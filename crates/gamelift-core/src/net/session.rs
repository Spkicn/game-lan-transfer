//! 传输请求：发送方发起，接收方选目录并同意后才开始搬运
//!
//! 数据仍走既有的分块传输通道，同意之后由接收方拉取
//! 请求本身走一条短连接：发送方发 [`TransferRequest`]，接收方回 [`Decision`]

use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::{Error, Result};

use super::frame::{read_frame, write_frame, Frame};
use super::protocol::{self, Message, RequestItem};
use super::{io_error, lock, IO_POLL_INTERVAL};

/// 请求监听默认端口
pub const REQUEST_PORT: u16 = 27102;

/// 建立请求连接的等待时间
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// 接收方等待用户决定的最长时间
pub const DECISION_TIMEOUT: Duration = Duration::from_secs(120);

/// 发送方等待答复的最长时间
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(140);

/// 一次传输请求
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferRequest {
    /// 发送方主机名
    pub sender_name: String,
    /// 待传内容清单
    pub items: Vec<RequestItem>,
    /// 内容总字节数
    pub total_bytes: u64,
    /// 发送方传输服务端口
    pub transfer_port: u16,
    /// 配对码
    pub pairing: Option<String>,
    /// 拉取时使用的内容名
    pub want: String,
    /// 内容所属平台，接收方按它决定认领文件写到哪
    pub platform: Option<String>,
}

impl TransferRequest {
    /// 转成协议消息
    #[must_use]
    pub fn to_message(&self) -> Message {
        Message::TransferRequest {
            sender_name: self.sender_name.clone(),
            items: self.items.clone(),
            total_bytes: self.total_bytes,
            transfer_port: self.transfer_port,
            pairing: self.pairing.clone(),
            want: self.want.clone(),
            platform: self.platform.clone(),
        }
    }
}

/// 接收方的决定
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// 同意，内容落到该目录
    Approve {
        /// 目标目录
        dest: String,
    },
    /// 拒绝
    Reject {
        /// 给发送方看的原因
        reason: String,
    },
}

impl Decision {
    /// 转成协议消息
    #[must_use]
    fn to_message(&self) -> Message {
        match self {
            Self::Approve { dest } => Message::TransferDecision {
                accepted: true,
                dest: Some(dest.clone()),
                message: String::new(),
            },
            Self::Reject { reason } => Message::TransferDecision {
                accepted: false,
                dest: None,
                message: reason.clone(),
            },
        }
    }

    /// 从协议消息还原
    fn from_message(message: Message) -> Self {
        match message {
            Message::TransferDecision {
                accepted,
                dest,
                message,
            } => {
                if accepted {
                    dest.map_or_else(
                        || Self::Reject {
                            reason: "对端未给出目标目录".to_owned(),
                        },
                        |dest| Self::Approve { dest },
                    )
                } else {
                    Self::Reject {
                        reason: if message.is_empty() {
                            "对端拒绝了本次传输".to_owned()
                        } else {
                            message
                        },
                    }
                }
            }
            Message::Rejected { kind, message } => Self::Reject {
                reason: if message.is_empty() {
                    format!("对端拒绝：{kind:?}")
                } else {
                    message
                },
            },
            other => Self::Reject {
                reason: format!("对端返回了意外消息: {other:?}"),
            },
        }
    }
}

/// 发送一次传输请求并等待答复
///
/// # Errors
///
/// 连接失败、超时或协议错误时返回说明
pub fn request_transfer(
    peer: SocketAddr,
    request: &TransferRequest,
    timeout: Duration,
) -> Result<Decision> {
    let stream = TcpStream::connect_timeout(&peer, CONNECT_TIMEOUT)
        .map_err(|err| Error::Io(format!("连接 {peer} 失败: {err}")))?;
    stream.set_read_timeout(Some(timeout)).map_err(io_error)?;
    stream.set_write_timeout(Some(timeout)).map_err(io_error)?;
    let mut reader = std::io::BufReader::new(stream.try_clone().map_err(io_error)?);
    let mut writer = std::io::BufWriter::new(stream);
    write_frame(
        &mut writer,
        &Frame::control(protocol::encode(&request.to_message())?),
    )?;
    let Some(frame) = read_frame(&mut reader)? else {
        return Err(Error::Protocol("对端没有给出答复就关闭了连接".to_owned()));
    };
    if !frame.is_control() {
        return Err(Error::Protocol("答复不是控制帧".to_owned()));
    }
    Ok(Decision::from_message(protocol::decode(&frame.payload)?))
}

/// 接收端收到的一条请求
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingRequest {
    /// 请求编号，回应时使用
    pub id: u64,
    /// 发送方地址
    pub from: SocketAddr,
    /// 请求内容
    pub request: TransferRequest,
}

/// 共享的待处理状态
#[derive(Debug, Default)]
struct Pending {
    /// 等待界面处理的请求
    queue: Mutex<VecDeque<IncomingRequest>>,
    /// 请求编号到答复通道
    waiting: Mutex<HashMap<u64, Sender<Decision>>>,
}

/// 接收端请求监听器，只绑调用方给出的直连地址
#[derive(Debug)]
pub struct RequestListener {
    local_addr: SocketAddr,
    pending: Arc<Pending>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl RequestListener {
    /// 启动监听
    ///
    /// # Errors
    ///
    /// 绑定地址不明确或端口被占用时返回错误
    pub fn start(bind: SocketAddr, pairing: Option<String>) -> Result<Self> {
        if bind.ip().is_unspecified() {
            return Err(Error::Protocol(
                "接收请求必须绑定明确的直连地址，不能监听全部接口".to_owned(),
            ));
        }
        let listener =
            TcpListener::bind(bind).map_err(|err| Error::Io(format!("监听 {bind} 失败: {err}")))?;
        listener
            .set_nonblocking(true)
            .map_err(|err| Error::Io(format!("设置非阻塞失败: {err}")))?;
        let local_addr = listener
            .local_addr()
            .map_err(|err| Error::Io(format!("读取监听地址失败: {err}")))?;
        let pending = Arc::new(Pending::default());
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_pending = Arc::clone(&pending);
        let thread_shutdown = Arc::clone(&shutdown);
        let handle = thread::Builder::new()
            .name("gamelift-request".to_owned())
            .spawn(move || {
                accept_requests(
                    &listener,
                    &thread_pending,
                    &thread_shutdown,
                    pairing.as_deref(),
                );
            })
            .map_err(io_error)?;
        Ok(Self {
            local_addr,
            pending,
            shutdown,
            handle: Some(handle),
        })
    }

    /// 实际监听地址
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// 取一条待处理请求，超时返回 `None`
    ///
    /// # Errors
    ///
    /// 心跳线程异常时返回说明
    #[must_use]
    pub fn poll(&self, timeout: Duration) -> Option<IncomingRequest> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(found) = lock(&self.pending.queue).pop_front() {
                return Some(found);
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            thread::sleep(IO_POLL_INTERVAL);
        }
    }

    /// 答复一条请求，超时后连接已经关闭，返回错误
    ///
    /// # Errors
    ///
    /// 请求不存在或答复通道已关闭时返回错误
    pub fn respond(&self, id: u64, decision: &Decision) -> Result<()> {
        let sender = lock(&self.pending.waiting).remove(&id);
        let Some(sender) = sender else {
            return Err(Error::Protocol(format!("请求 {id} 已失效")));
        };
        sender
            .send(decision.clone())
            .map_err(|_| Error::Protocol(format!("请求 {id} 已断开")))
    }

    /// 停止监听并等待接受线程退出
    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for RequestListener {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// 接受连接并处理请求
fn accept_requests(
    listener: &TcpListener,
    pending: &Arc<Pending>,
    shutdown: &AtomicBool,
    pairing: Option<&str>,
) {
    let next_id = AtomicU64::new(1);
    while !shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, from)) => {
                if stream.set_nonblocking(false).is_err() {
                    continue;
                }
                let pending = Arc::clone(pending);
                let expected = pairing.map(str::to_owned);
                let id = next_id.fetch_add(1, Ordering::Relaxed);
                let spawned = thread::Builder::new()
                    .name("gamelift-request-session".to_owned())
                    .spawn(move || {
                        let _ = handle_request(stream, from, id, &pending, expected.as_deref());
                    });
                if spawned.is_err() {
                    thread::sleep(IO_POLL_INTERVAL);
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(IO_POLL_INTERVAL);
            }
            Err(_) => thread::sleep(IO_POLL_INTERVAL),
        }
    }
}

/// 处理一条请求连接：读请求、交给界面、回答复
fn handle_request(
    stream: TcpStream,
    from: SocketAddr,
    id: u64,
    pending: &Arc<Pending>,
    expected_pairing: Option<&str>,
) -> Result<()> {
    let mut reader = std::io::BufReader::new(stream.try_clone().map_err(io_error)?);
    let mut writer = std::io::BufWriter::new(stream);
    let Some(frame) = read_frame(&mut reader)? else {
        return Ok(());
    };
    let Message::TransferRequest {
        sender_name,
        items,
        total_bytes,
        transfer_port,
        pairing,
        want,
        platform,
    } = protocol::decode(&frame.payload)?
    else {
        return reply(
            &mut writer,
            &Decision::Reject {
                reason: "首帧不是传输请求".to_owned(),
            },
        );
    };
    if let Some(expected) = expected_pairing {
        if pairing.as_deref() != Some(expected) {
            return reply(
                &mut writer,
                &Decision::Reject {
                    reason: "配对码不匹配，请核对发送方显示的 6 位数字".to_owned(),
                },
            );
        }
    }
    let (tx, rx): (Sender<Decision>, Receiver<Decision>) = mpsc::channel();
    lock(&pending.waiting).insert(id, tx);
    lock(&pending.queue).push_back(IncomingRequest {
        id,
        from,
        request: TransferRequest {
            sender_name,
            items,
            total_bytes,
            transfer_port,
            pairing,
            want,
            platform,
        },
    });
    let decision = match rx.recv_timeout(DECISION_TIMEOUT) {
        Ok(decision) => decision,
        Err(RecvTimeoutError::Timeout) => Decision::Reject {
            reason: "对端迟迟没有回应，已取消".to_owned(),
        },
        Err(RecvTimeoutError::Disconnected) => Decision::Reject {
            reason: "本机取消了这次传输".to_owned(),
        },
    };
    lock(&pending.waiting).remove(&id);
    reply(&mut writer, &decision)
}

/// 写回答复
fn reply(writer: &mut impl std::io::Write, decision: &Decision) -> Result<()> {
    write_frame(
        writer,
        &Frame::control(protocol::encode(&decision.to_message())?),
    )
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn request() -> TransferRequest {
        TransferRequest {
            sender_name: "laptop".to_owned(),
            items: vec![RequestItem {
                name: "demo".to_owned(),
                is_dir: true,
                bytes: 2048,
            }],
            total_bytes: 2048,
            transfer_port: 27101,
            pairing: Some("123456".to_owned()),
            want: "demo".to_owned(),
            platform: Some("steam".to_owned()),
        }
    }

    #[test]
    fn approve_roundtrip() {
        let mut listener = RequestListener::start(
            SocketAddr::from(([127, 0, 0, 1], 0)),
            Some("123456".to_owned()),
        )
        .expect("listen");
        let addr = listener.local_addr();
        let sender = thread::spawn(move || {
            request_transfer(addr, &request(), Duration::from_secs(10)).expect("send")
        });
        let incoming = listener.poll(Duration::from_secs(5)).expect("incoming");
        assert_eq!(incoming.request.sender_name, "laptop");
        assert_eq!(incoming.request.items.len(), 1);
        assert_eq!(incoming.request.transfer_port, 27101);
        listener
            .respond(
                incoming.id,
                &Decision::Approve {
                    dest: "D:/games".to_owned(),
                },
            )
            .expect("respond");
        let decision = sender.join().expect("join");
        assert_eq!(
            decision,
            Decision::Approve {
                dest: "D:/games".to_owned()
            }
        );
        listener.shutdown();
    }

    #[test]
    fn reject_roundtrip() {
        let mut listener =
            RequestListener::start(SocketAddr::from(([127, 0, 0, 1], 0)), None).expect("listen");
        let addr = listener.local_addr();
        let sender = thread::spawn(move || {
            request_transfer(addr, &request(), Duration::from_secs(10)).expect("send")
        });
        let incoming = listener.poll(Duration::from_secs(5)).expect("incoming");
        listener
            .respond(
                incoming.id,
                &Decision::Reject {
                    reason: "空间不够".to_owned(),
                },
            )
            .expect("respond");
        let decision = sender.join().expect("join");
        assert_eq!(
            decision,
            Decision::Reject {
                reason: "空间不够".to_owned()
            }
        );
        listener.shutdown();
    }

    #[test]
    fn wrong_pairing_is_rejected() {
        let mut listener = RequestListener::start(
            SocketAddr::from(([127, 0, 0, 1], 0)),
            Some("123456".to_owned()),
        )
        .expect("listen");
        let addr = listener.local_addr();
        let mut wrong = request();
        wrong.pairing = Some("000000".to_owned());
        let decision = request_transfer(addr, &wrong, Duration::from_secs(10)).expect("send");
        assert!(
            matches!(decision, Decision::Reject { .. }),
            "got {decision:?}"
        );
        // 被拒绝的请求不应进入界面队列
        assert!(listener.poll(Duration::from_millis(200)).is_none());
        listener.shutdown();
    }

    #[test]
    fn unspecified_bind_is_rejected() {
        let err = RequestListener::start(SocketAddr::from(([0, 0, 0, 0], 0)), None)
            .expect_err("must reject");
        assert!(matches!(err, Error::Protocol(_)), "got {err:?}");
    }
}
