//! 源端会话服务：监听指定直连地址，按请求下发清单与分块
//!
//! 只暴露构造时给出的根目录，且拒绝监听全部接口
//! 分块摘要由源端即时计算并随块下发，接收端据此判断传输是否损坏

use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crate::{Error, Result};

use super::frame::{read_frame, write_frame, Frame};
use super::paths::{join_within, sanitize_relative};
use super::protocol::{self, ClaimFile, FileEntry, Message, RejectKind, PROTOCOL_VERSION};
use super::{DIGEST_BYTES, IO_POLL_INTERVAL};

/// 源端启动参数
#[derive(Debug, Clone)]
pub struct HostOptions {
    /// 要暴露的根目录
    pub root: PathBuf,
    /// 监听地址，必须是明确的直连地址
    pub bind: SocketAddr,
    /// 配对码，`None` 表示不校验
    pub pairing: Option<String>,
    /// 清单标题
    pub title: String,
    /// 平台标识
    pub platform: String,
    /// 目标根目录名，默认取根目录的目录名
    pub root_name: Option<String>,
    /// 内容落盘后由目标端写入的认领文件
    pub claim_files: Vec<ClaimFile>,
    /// 分块大小
    pub chunk_bytes: u32,
}

/// 源端对外暴露的一个文件
#[derive(Debug, Clone)]
struct ServedFile {
    /// 相对根目录的路径
    relative: String,
    /// 字节数
    size: u64,
}

/// 会话累计计数，供调用方展示与测试断言
#[derive(Debug, Default)]
struct Counters {
    /// 已下发的分块负载字节数
    bytes_sent: AtomicU64,
    /// 已接受的会话数
    sessions: AtomicU64,
}

/// 一次会话服务的共享上下文
struct Session {
    options: Arc<HostOptions>,
    files: Vec<ServedFile>,
    counters: Arc<Counters>,
}

/// 正在运行的源端服务，析构时自动停止监听
#[derive(Debug)]
pub struct Host {
    local_addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    counters: Arc<Counters>,
    handle: Option<JoinHandle<()>>,
}

impl Host {
    /// 启动源端服务
    ///
    /// # Errors
    ///
    /// 绑定地址不明确、根目录不存在或为空、端口被占用时返回错误
    pub fn start(mut options: HostOptions) -> Result<Self> {
        if options.bind.ip().is_unspecified() {
            return Err(Error::Protocol(
                "源端必须绑定明确的直连地址，不能监听全部接口".to_owned(),
            ));
        }
        if !options.root.is_dir() {
            return Err(Error::Io(format!(
                "源目录不存在: {}",
                options.root.display()
            )));
        }
        let files = collect_files(&options.root)?;
        if files.is_empty() {
            return Err(Error::Io("源目录里没有可传输的文件".to_owned()));
        }
        if options.root_name.is_none() {
            options.root_name = Some(default_root_name(&options.root));
        }
        let options = Arc::new(options);
        let counters = Arc::new(Counters::default());
        let session = Arc::new(Session {
            options: Arc::clone(&options),
            files,
            counters: Arc::clone(&counters),
        });
        let listener = TcpListener::bind(options.bind)
            .map_err(|err| Error::Io(format!("监听 {} 失败: {err}", options.bind)))?;
        listener
            .set_nonblocking(true)
            .map_err(|err| Error::Io(format!("设置非阻塞失败: {err}")))?;
        let local_addr = listener
            .local_addr()
            .map_err(|err| Error::Io(format!("读取监听地址失败: {err}")))?;
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);
        let handle = thread::Builder::new()
            .name("gamelift-host".to_owned())
            .spawn(move || accept_loop(&listener, &session, &thread_shutdown))
            .map_err(super::io_error)?;
        Ok(Self {
            local_addr,
            shutdown,
            counters,
            handle: Some(handle),
        })
    }

    /// 实际监听地址
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// 已下发的分块负载字节数
    #[must_use]
    pub fn bytes_sent(&self) -> u64 {
        self.counters.bytes_sent.load(Ordering::Relaxed)
    }

    /// 已接受的会话数
    #[must_use]
    pub fn sessions_served(&self) -> u64 {
        self.counters.sessions.load(Ordering::Relaxed)
    }

    /// 停止监听并等待接受线程退出
    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// 递归收集根目录下的普通文件
///
/// # Errors
///
/// 目录不可读时返回 [`Error::Io`]
fn collect_files(root: &Path) -> Result<Vec<ServedFile>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|err| Error::Io(format!("读取目录 {} 失败: {err}", dir.display())))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            let Ok(stripped) = path.strip_prefix(root) else {
                continue;
            };
            let Some(relative) = stripped.to_str() else {
                continue;
            };
            out.push(ServedFile {
                relative: relative.replace('\\', "/"),
                size: metadata.len(),
            });
        }
    }
    out.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(out)
}

/// 根目录名，取不到时退回固定名
fn default_root_name(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "transfer".to_owned())
}

/// 接受连接并为每个会话开一个线程
fn accept_loop(listener: &TcpListener, session: &Arc<Session>, shutdown: &AtomicBool) {
    while !shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _peer)) => {
                // Windows 上 accept 出来的套接字会继承监听套接字的非阻塞模式，这里必须显式改回阻塞
                if stream.set_nonblocking(false).is_err() {
                    continue;
                }
                let session = Arc::clone(session);
                let spawned = thread::Builder::new()
                    .name("gamelift-session".to_owned())
                    .spawn(move || {
                        let _ = handle_connection(stream, &session);
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

/// 处理一条会话连接：握手后按请求下发分块
fn handle_connection(stream: TcpStream, session: &Arc<Session>) -> Result<()> {
    stream
        .set_nodelay(true)
        .map_err(|err| Error::Io(format!("设置 TCP 参数失败: {err}")))?;
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|err| Error::Io(format!("复制连接失败: {err}")))?,
    );
    let mut writer = BufWriter::new(stream);
    let Some(hello) = read_message(&mut reader)? else {
        return Ok(());
    };
    let Message::Hello {
        version,
        pairing,
        need_manifest,
        ..
    } = hello
    else {
        return send_rejected(&mut writer, RejectKind::Version, "首帧必须是问候");
    };
    session.counters.sessions.fetch_add(1, Ordering::Relaxed);
    if version != PROTOCOL_VERSION {
        return send_rejected(
            &mut writer,
            RejectKind::Version,
            &format!("协议版本 {version} 与源端 {PROTOCOL_VERSION} 不一致"),
        );
    }
    if let Some(expected) = &session.options.pairing {
        if pairing.as_deref() != Some(expected.as_str()) {
            return send_rejected(&mut writer, RejectKind::Pairing, "配对码不匹配");
        }
    }
    if need_manifest {
        send_message(&mut writer, &manifest_message(session))?;
    } else {
        send_message(&mut writer, &Message::Ready)?;
    }
    serve_requests(&mut reader, &mut writer, session)
}

/// 循环处理分块请求，收到 `Bye` 或对端关闭时结束
fn serve_requests(
    reader: &mut impl Read,
    writer: &mut impl Write,
    session: &Arc<Session>,
) -> Result<()> {
    while let Some(frame) = read_frame(reader)? {
        if !frame.is_control() {
            return Err(Error::Protocol("源端只接受控制帧".to_owned()));
        }
        match protocol::decode(&frame.payload)? {
            Message::RequestChunk { file, offset, len } => {
                serve_chunk(writer, session, file, offset, len)?;
            }
            Message::Bye => break,
            other => {
                return Err(Error::Protocol(format!("源端不接受该消息: {other:?}")));
            }
        }
    }
    Ok(())
}

/// 下发一个分块，负载为 `blake3` 摘要加数据
fn serve_chunk(
    writer: &mut impl Write,
    session: &Arc<Session>,
    file: u32,
    offset: u64,
    len: u32,
) -> Result<()> {
    let Some(entry) = usize::try_from(file)
        .ok()
        .and_then(|index| session.files.get(index))
    else {
        return send_rejected(writer, RejectKind::NotFound, "文件下标越界");
    };
    let length = u64::from(len);
    if len == 0 || len > super::MAX_CHUNK_BYTES || offset.saturating_add(length) > entry.size {
        return send_rejected(writer, RejectKind::NotFound, "分块范围非法");
    }
    let relative = sanitize_relative(&entry.relative)?;
    let path = join_within(&session.options.root, &relative)?;
    let data = match read_chunk(&path, offset, len) {
        Ok(data) => data,
        Err(err) => {
            return send_rejected(writer, RejectKind::Source, &err.to_string());
        }
    };
    let digest = blake3::hash(&data);
    let mut payload = Vec::with_capacity(DIGEST_BYTES + data.len());
    payload.extend_from_slice(digest.as_bytes());
    payload.extend_from_slice(&data);
    write_frame(writer, &Frame::chunk(payload))?;
    session
        .counters
        .bytes_sent
        .fetch_add(length, Ordering::Relaxed);
    Ok(())
}

/// 读取文件的指定区间
///
/// # Errors
///
/// 文件缺失、定位失败或区间不完整时返回 [`Error::Io`]
fn read_chunk(path: &Path, offset: u64, len: u32) -> Result<Vec<u8>> {
    let mut handle = std::fs::File::open(path)
        .map_err(|err| Error::Io(format!("打开 {} 失败: {err}", path.display())))?;
    handle
        .seek(SeekFrom::Start(offset))
        .map_err(|err| Error::Io(format!("定位 {} 失败: {err}", path.display())))?;
    let mut data = vec![0u8; usize::try_from(len).unwrap_or(0)];
    handle
        .read_exact(&mut data)
        .map_err(|err| Error::Io(format!("读取 {} 失败: {err}", path.display())))?;
    Ok(data)
}

/// 组装清单消息
fn manifest_message(session: &Arc<Session>) -> Message {
    let files: Vec<FileEntry> = session
        .files
        .iter()
        .map(|file| FileEntry {
            path: file.relative.clone(),
            size: file.size,
        })
        .collect();
    let total_bytes = session.files.iter().map(|file| file.size).sum();
    Message::Manifest {
        title: session.options.title.clone(),
        platform: session.options.platform.clone(),
        root_name: session.options.root_name.clone().unwrap_or_default(),
        files,
        total_bytes,
        claim_files: session.options.claim_files.clone(),
    }
}

/// 读取一条控制消息，对端正常关闭时返回 `None`
fn read_message(reader: &mut impl Read) -> Result<Option<Message>> {
    let Some(frame) = read_frame(reader)? else {
        return Ok(None);
    };
    if !frame.is_control() {
        return Err(Error::Protocol("握手阶段期望控制帧".to_owned()));
    }
    protocol::decode(&frame.payload).map(Some)
}

/// 发送一条控制消息
///
/// # Errors
///
/// 编码或写入失败时返回错误
fn send_message(writer: &mut impl Write, message: &Message) -> Result<()> {
    write_frame(writer, &Frame::control(protocol::encode(message)?))
}

/// 发送拒绝消息
fn send_rejected(writer: &mut impl Write, kind: RejectKind, message: &str) -> Result<()> {
    send_message(
        writer,
        &Message::Rejected {
            kind,
            message: message.to_owned(),
        },
    )
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gamelift-host-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn options(root: PathBuf, bind: SocketAddr) -> HostOptions {
        HostOptions {
            root,
            bind,
            pairing: None,
            title: "示例".to_owned(),
            platform: "steam".to_owned(),
            root_name: None,
            claim_files: Vec::new(),
            chunk_bytes: 1024,
        }
    }

    #[test]
    fn collect_files_walks_nested_dirs_sorted() {
        let root = temp_dir("walk");
        std::fs::create_dir_all(root.join("bin/win64")).expect("mkdir");
        std::fs::write(root.join("bin/win64/game.exe"), b"x").expect("write");
        std::fs::write(root.join("readme.txt"), b"y").expect("write");
        let files = collect_files(&root).expect("walk");
        let paths: Vec<&str> = files.iter().map(|file| file.relative.as_str()).collect();
        assert_eq!(paths, vec!["bin/win64/game.exe", "readme.txt"]);
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn start_rejects_unspecified_bind() {
        let root = temp_dir("unspecified");
        std::fs::write(root.join("a.bin"), b"data").expect("write");
        let err = Host::start(options(
            root,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        ))
        .expect_err("must reject");
        assert!(matches!(err, Error::Protocol(_)), "got {err:?}");
    }

    #[test]
    fn start_rejects_empty_root() {
        let root = temp_dir("empty");
        let err = Host::start(options(
            root,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        ))
        .expect_err("must reject");
        assert!(matches!(err, Error::Io(_)), "got {err:?}");
    }

    #[test]
    fn start_binds_loopback_and_reports_counts() {
        let root = temp_dir("start");
        std::fs::write(root.join("a.bin"), b"data").expect("write");
        let mut host = Host::start(options(
            root.clone(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        ))
        .expect("start");
        assert!(host.local_addr().port() > 0);
        assert_eq!(host.bytes_sent(), 0);
        assert_eq!(host.sessions_served(), 0);
        host.shutdown();
        std::fs::remove_dir_all(&root).expect("cleanup");
    }
}
