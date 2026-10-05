//! 源端会话服务：监听指定直连地址，按请求下发清单与分块
//!
//! 只暴露构造时给出的根目录，且拒绝监听全部接口
//! 分块摘要由源端即时计算并随块下发，接收端据此判断传输是否损坏

use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crate::{Error, Result};

use super::diff;
use super::frame::{read_frame, write_frame, Frame, KIND_CONTROL, KIND_HASHES, KIND_PLAN};
use super::paths::{join_within, sanitize_relative};
use super::protocol::{self, ClaimFile, FileEntry, Message, RejectKind, PROTOCOL_VERSION};
use super::{DIGEST_BYTES, IO_POLL_INTERVAL};

/// 源端启动参数
#[derive(Debug, Clone)]
pub struct HostOptions {
    /// 要暴露的根目录
    pub root: PathBuf,
    /// 附加内容根，目标端看到的是各自的 `<名字>/` 这一层；为空时行为与单根完全一致
    pub extra_roots: Vec<ExtraRoot>,
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
    /// 主根是否也以 `<root_name>/` 前缀进清单，用于一次搬运多个互不相干的条目
    pub wrap_root: bool,
    /// 内容落盘后由目标端写入的认领文件
    pub claim_files: Vec<ClaimFile>,
    /// 分块大小
    pub chunk_bytes: u32,
}

/// 一个附加内容根，用于一次搬运多份互不相干的文件夹或文件
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraRoot {
    /// 顶层名字，必须是单个路径段
    pub name: String,
    /// 本机路径，可以是文件也可以是目录
    pub path: PathBuf,
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
        if !options.root.is_dir() && !options.root.is_file() {
            return Err(Error::Io(format!(
                "源目录不存在: {}",
                options.root.display()
            )));
        }
        for extra in &options.extra_roots {
            if sanitize_relative(&extra.name).is_err() {
                return Err(Error::PathEscape(extra.name.clone()));
            }
            if !extra.path.exists() {
                return Err(Error::Io(format!(
                    "附加内容不存在: {}",
                    extra.path.display()
                )));
            }
        }
        if options.root_name.is_none() {
            options.root_name = Some(default_root_name(&options.root));
        }
        let files = collect_files(&options)?;
        if files.is_empty() {
            return Err(Error::Io("源目录里没有可传输的文件".to_owned()));
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

/// 递归收集全部内容根下的普通文件，附加根带 `<名字>/` 前缀
///
/// # Errors
///
/// 目录不可读时返回 [`Error::Io`]
fn collect_files(options: &HostOptions) -> Result<Vec<ServedFile>> {
    let mut out = Vec::new();
    let primary_prefix = if options.wrap_root {
        options.root_name.clone().unwrap_or_default()
    } else {
        String::new()
    };
    walk_into(&options.root, primary_prefix.as_str(), &mut out)?;
    for extra in &options.extra_roots {
        walk_into(&extra.path, &extra.name, &mut out)?;
    }
    out.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(out)
}

/// 把一个根下的普通文件收进清单，`prefix` 非空时加一层顶层名字
fn walk_into(root: &Path, prefix: &str, out: &mut Vec<ServedFile>) -> Result<()> {
    // 单个文件也能作为内容根
    if root.is_file() {
        let name = if prefix.is_empty() {
            root.file_name().map_or_else(
                || "file".to_owned(),
                |name| name.to_string_lossy().into_owned(),
            )
        } else {
            prefix.to_owned()
        };
        let size = std::fs::metadata(root).map_or(0, |meta| meta.len());
        out.push(ServedFile {
            relative: name,
            size,
        });
        return Ok(());
    }
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
            let Some(relative) = relative_name(&path, root, prefix) else {
                continue;
            };
            out.push(ServedFile {
                relative,
                size: metadata.len(),
            });
        }
    }
    Ok(())
}

/// 把路径转成清单里的相对路径，`prefix` 非空时作为顶层名字
fn relative_name(path: &Path, root: &Path, prefix: &str) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?.to_str()?.replace('\\', "/");
    Some(if prefix.is_empty() {
        relative
    } else {
        format!("{prefix}/{relative}")
    })
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
        match frame.kind {
            KIND_CONTROL => match protocol::decode(&frame.payload)? {
                Message::RequestChunk { file, offset, len } => {
                    serve_chunk(writer, session, file, offset, len)?;
                }
                Message::Bye => return Ok(()),
                other => {
                    return Err(Error::Protocol(format!("源端不接受该消息: {other:?}")));
                }
            },
            KIND_HASHES => serve_diff(writer, session, &frame.payload)?,
            other => {
                return Err(Error::Protocol(format!("源端不认识帧类型 {other}")));
            }
        }
    }
    Ok(())
}

/// 差异预交换：按接收端给出的旧块摘要，只列出需要下发的块
fn serve_diff(writer: &mut impl Write, session: &Arc<Session>, payload: &[u8]) -> Result<()> {
    let (file, hashes) = diff::decode_hashes(payload)?;
    let Some(entry) = usize::try_from(file)
        .ok()
        .and_then(|index| session.files.get(index))
    else {
        return send_rejected(writer, RejectKind::NotFound, "文件下标越界");
    };
    let path = resolve_path(&session.options, entry.relative.as_str())?;
    let source = match diff::index_file(&path) {
        Ok(source) => source,
        Err(err) => return send_rejected(writer, RejectKind::Source, &err.to_string()),
    };
    let wanted: HashMap<[u8; DIGEST_BYTES], u32> = hashes
        .iter()
        .enumerate()
        .map(|(index, hash)| (*hash, u32::try_from(index).unwrap_or(u32::MAX)))
        .collect();
    let plan = diff::build_plan(&source, &wanted);
    write_frame(
        writer,
        &Frame {
            kind: KIND_PLAN,
            payload: diff::encode_plan(file, &plan),
        },
    )
}

/// 把清单里的相对路径解析到实际文件，附加根按顶层名字优先匹配
///
/// # Errors
///
/// 路径非法或越出内容根时返回错误
fn resolve_path(options: &HostOptions, relative: &str) -> Result<PathBuf> {
    let sanitized = sanitize_relative(relative)?;
    let mut components = sanitized.components();
    if let Some(Component::Normal(first)) = components.next() {
        if let Some(name) = first.to_str() {
            if options.wrap_root && options.root_name.as_deref() == Some(name) {
                let rest: PathBuf = components.collect();
                if rest.as_os_str().is_empty() {
                    return Ok(options.root.clone());
                }
                return join_within(&options.root, &rest);
            }
            if let Some(extra) = options.extra_roots.iter().find(|root| root.name == name) {
                let rest: PathBuf = components.collect();
                if rest.as_os_str().is_empty() {
                    return Ok(extra.path.clone());
                }
                return join_within(&extra.path, &rest);
            }
        }
    }
    join_within(&options.root, &sanitized)
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
    let path = resolve_path(&session.options, entry.relative.as_str())?;
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
            extra_roots: Vec::new(),
            wrap_root: false,
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
        let files = collect_files(&options(
            root.clone(),
            SocketAddr::from(([127, 0, 0, 1], 0)),
        ))
        .expect("walk");
        let paths: Vec<&str> = files.iter().map(|file| file.relative.as_str()).collect();
        assert_eq!(paths, vec!["bin/win64/game.exe", "readme.txt"]);
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn extra_roots_are_prefixed_and_resolved() {
        let root = temp_dir("multi-root");
        let extra_dir = temp_dir("multi-extra");
        std::fs::write(root.join("a.bin"), b"primary").expect("write");
        std::fs::write(extra_dir.join("b.bin"), b"extra").expect("write");
        let mut host_options = options(root.clone(), SocketAddr::from(([127, 0, 0, 1], 0)));
        host_options.extra_roots = vec![ExtraRoot {
            name: "附加".to_owned(),
            path: extra_dir.clone(),
        }];
        let files = collect_files(&host_options).expect("walk");
        let paths: Vec<&str> = files.iter().map(|file| file.relative.as_str()).collect();
        assert_eq!(paths, vec!["a.bin", "附加/b.bin"]);
        let resolved = resolve_path(&host_options, "附加/b.bin").expect("resolve");
        assert_eq!(resolved, extra_dir.join("b.bin"));
        let primary = resolve_path(&host_options, "a.bin").expect("resolve");
        assert_eq!(primary, root.join("a.bin"));
        std::fs::remove_dir_all(&root).expect("cleanup");
        std::fs::remove_dir_all(&extra_dir).expect("cleanup");
    }

    #[test]
    fn wrapped_primary_root_keeps_its_name() {
        let root = temp_dir("wrapped");
        std::fs::create_dir_all(root.join("bin")).expect("mkdir");
        std::fs::write(root.join("bin/a.bin"), b"x").expect("write");
        let mut host_options = options(root.clone(), SocketAddr::from(([127, 0, 0, 1], 0)));
        host_options.root_name = Some("demo".to_owned());
        host_options.wrap_root = true;
        let files = collect_files(&host_options).expect("walk");
        assert_eq!(files[0].relative, "demo/bin/a.bin");
        let resolved = resolve_path(&host_options, "demo/bin/a.bin").expect("resolve");
        assert_eq!(resolved, root.join("bin/a.bin"));
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn single_file_root_is_served_by_name() {
        let root = temp_dir("single-file");
        let file = root.join("payload.bin");
        std::fs::write(&file, b"data").expect("write");
        let mut host_options = options(root.clone(), SocketAddr::from(([127, 0, 0, 1], 0)));
        host_options.root = file.clone();
        let files = collect_files(&host_options).expect("walk");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].relative, "payload.bin");
        assert_eq!(files[0].size, 4);
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
