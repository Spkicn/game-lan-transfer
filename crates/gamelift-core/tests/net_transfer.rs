//! 局域网传输集成测试
//!
//! 全部绑 `127.0.0.1`，端口从 18000 起顺次分配，不依赖外网

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::cast_precision_loss)]

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use gamelift_core::net::client::{recv, RecvOptions};
use gamelift_core::net::frame::{read_frame, write_frame, Frame};
use gamelift_core::net::protocol::{self, FileEntry, Message};
use gamelift_core::net::resume::{self, FileState, TransferState};
use gamelift_core::net::server::{Host, HostOptions};
use gamelift_core::{Error, Result};

/// 测试端口分配器，单调递增避免并行用例抢占同一端口
static NEXT_PORT: AtomicU16 = AtomicU16::new(18000);

/// 取一个可用端口
fn free_port() -> u16 {
    loop {
        let port = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
}

/// 回环地址
fn addr(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

/// 独立临时目录
fn temp_dir(tag: &str) -> PathBuf {
    static SEQ: AtomicU16 = AtomicU16::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("gamelift-it-{tag}-{}-{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

/// 确定性伪随机内容，用来发现错位写入
fn pattern(len: usize) -> Vec<u8> {
    (0..len)
        .map(|index| u8::try_from(index % 251).unwrap_or(0))
        .collect()
}

/// 读取整个文件
fn read_file(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|err| panic!("读取 {} 失败: {err}", path.display()))
}

/// 源端参数
fn host_options(root: &Path, port: u16, chunk_bytes: u32) -> HostOptions {
    HostOptions {
        root: root.to_path_buf(),
        bind: addr(port),
        pairing: None,
        title: "示例游戏".to_owned(),
        platform: "generic".to_owned(),
        root_name: Some("demo".to_owned()),
        claim_files: Vec::new(),
        chunk_bytes,
    }
}

/// 接收参数
fn recv_options(peer: SocketAddr, dest: &Path, chunk_bytes: u32) -> RecvOptions {
    RecvOptions {
        peer,
        pairing: None,
        want: "demo".to_owned(),
        dest_parent: dest.to_path_buf(),
        claim_root: Some(dest.to_path_buf()),
        streams: 3,
        chunk_bytes,
        cancel: None,
        force: false,
        old_root: None,
    }
}

#[test]
fn transfers_tree_byte_for_byte() {
    let src = temp_dir("tree-src");
    std::fs::create_dir_all(src.join("bin/win64")).expect("mkdir");
    std::fs::create_dir_all(src.join("素材")).expect("mkdir");
    let big = pattern(700 * 1024);
    std::fs::write(src.join("bin/win64/game.bin"), &big).expect("write");
    std::fs::write(src.join("素材/地图 01.dat"), pattern(1234)).expect("write");
    std::fs::write(src.join("empty.txt"), b"").expect("write");
    let expected_total = (big.len() + 1234) as u64;

    let mut host = Host::start(host_options(&src, free_port(), 64 * 1024)).expect("host");
    let dest = temp_dir("tree-dest");
    let options = recv_options(host.local_addr(), &dest, 64 * 1024);
    let mut ticks = 0u32;
    let outcome = recv(&options, &mut |stats| {
        ticks += 1;
        assert!(stats.bytes_done <= stats.bytes_total);
    })
    .expect("recv");

    assert_eq!(outcome.bytes_total, expected_total);
    assert_eq!(outcome.bytes_received, expected_total);
    assert_eq!(outcome.bytes_resumed, 0);
    assert!(!outcome.root.ends_with(".gamelift-part-demo"));

    assert_eq!(read_file(&outcome.root.join("bin/win64/game.bin")), big);
    assert_eq!(
        read_file(&outcome.root.join("素材/地图 01.dat")).len(),
        1234
    );
    assert_eq!(read_file(&outcome.root.join("empty.txt")).len(), 0);
    assert!(ticks >= 1, "至少回调一次进度");
    host.shutdown();
    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&dest);
}

#[test]
fn resume_sends_only_missing_chunks() {
    let src = temp_dir("resume-src");
    let chunk = 64 * 1024u32;
    let payload = pattern(chunk as usize * 5);
    std::fs::write(src.join("big.bin"), &payload).expect("write");

    let mut host = Host::start(host_options(&src, free_port(), chunk)).expect("host");
    let dest = temp_dir("resume-dest");

    // 预置暂存目录与续传状态：已完成前 3 块
    let staging = dest.join(".gamelift-part-demo");
    std::fs::create_dir_all(&staging).expect("mkdir");
    std::fs::write(staging.join("big.bin"), &payload[..chunk as usize * 3]).expect("write");
    let mut bitmap = vec![0u8; resume::bitmap_bytes(5)];
    for index in 0..3 {
        resume::bit_set(&mut bitmap, index);
    }
    let state = TransferState::new(
        "示例游戏".to_owned(),
        u64::from(chunk),
        vec![FileState {
            path: "big.bin".to_owned(),
            size: payload.len() as u64,
            bitmap: resume::encode_bitmap(&bitmap),
            spans: Vec::new(),
        }],
    );
    let state_path = resume::state_path(&dest, "demo");
    state.save(&state_path).expect("save state");

    let options = recv_options(host.local_addr(), &dest, chunk);
    let outcome = recv(&options, &mut |_| {}).expect("recv");

    let expected_missing = payload.len() as u64 - u64::from(chunk) * 3;
    assert_eq!(host.bytes_sent(), expected_missing, "已完成分块必须零重传");
    assert_eq!(outcome.bytes_resumed, u64::from(chunk) * 3);
    assert_eq!(outcome.bytes_received, expected_missing);
    assert_eq!(read_file(&outcome.root.join("big.bin")), payload);
    assert!(!state_path.exists(), "成功后应清掉续传状态");
    host.shutdown();
    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&dest);
}

#[test]
fn cancel_keeps_state_and_next_run_finishes() {
    let src = temp_dir("cancel-src");
    let payload = pattern(200 * 1024);
    std::fs::write(src.join("data.bin"), &payload).expect("write");

    let mut host = Host::start(host_options(&src, free_port(), 64 * 1024)).expect("host");
    let dest = temp_dir("cancel-dest");
    let cancel = Arc::new(AtomicBool::new(true));
    let mut options = recv_options(host.local_addr(), &dest, 64 * 1024);
    options.cancel = Some(Arc::clone(&cancel));

    let err = recv(&options, &mut |_| {}).expect_err("must cancel");
    assert!(matches!(err, Error::Cancelled), "got {err:?}");
    assert_eq!(host.bytes_sent(), 0, "取消后不应有数据下发");
    let state_path = resume::state_path(&dest, "demo");
    assert!(state_path.exists(), "取消后必须保留续传状态");

    cancel.store(false, Ordering::Relaxed);
    let options = recv_options(host.local_addr(), &dest, 64 * 1024);
    let outcome = recv(&options, &mut |_| {}).expect("resume");
    assert_eq!(read_file(&outcome.root.join("data.bin")), payload);
    assert!(!state_path.exists());
    host.shutdown();
    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&dest);
}

#[test]
fn pairing_code_is_enforced() {
    let src = temp_dir("pair-src");
    std::fs::write(src.join("data.bin"), pattern(4096)).expect("write");
    let port = free_port();
    let mut options = host_options(&src, port, 64 * 1024);
    options.pairing = Some("123456".to_owned());
    let mut host = Host::start(options).expect("host");
    let dest = temp_dir("pair-dest");

    let mut wrong = recv_options(host.local_addr(), &dest, 64 * 1024);
    wrong.pairing = Some("000000".to_owned());
    let err = recv(&wrong, &mut |_| {}).expect_err("must reject");
    assert!(matches!(err, Error::PairingRejected), "got {err:?}");

    let mut missing = recv_options(host.local_addr(), &dest, 64 * 1024);
    missing.pairing = None;
    let err = recv(&missing, &mut |_| {}).expect_err("must reject");
    assert!(matches!(err, Error::PairingRejected), "got {err:?}");

    let mut correct = recv_options(host.local_addr(), &dest, 64 * 1024);
    correct.pairing = Some("123456".to_owned());
    let outcome = recv(&correct, &mut |_| {}).expect("accept");
    assert_eq!(read_file(&outcome.root.join("data.bin")).len(), 4096);
    host.shutdown();
    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&dest);
}

#[test]
fn existing_destination_requires_force() {
    let src = temp_dir("force-src");
    std::fs::write(src.join("data.bin"), pattern(8192)).expect("write");
    let mut host = Host::start(host_options(&src, free_port(), 64 * 1024)).expect("host");
    let dest = temp_dir("force-dest");

    let options = recv_options(host.local_addr(), &dest, 64 * 1024);
    recv(&options, &mut |_| {}).expect("first run");
    let err = recv(&options, &mut |_| {}).expect_err("must refuse");
    assert!(matches!(err, Error::Io(_)), "got {err:?}");

    let mut forced = recv_options(host.local_addr(), &dest, 64 * 1024);
    forced.force = true;
    let outcome = recv(&forced, &mut |_| {}).expect("forced run");
    assert_eq!(read_file(&outcome.root.join("data.bin")).len(), 8192);
    host.shutdown();
    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&dest);
}

#[test]
fn manifest_with_traversal_path_is_rejected() {
    let payload = pattern(2048);
    let server = FakeServer::start(
        vec![FileEntry {
            path: "../evil.bin".to_owned(),
            size: payload.len() as u64,
        }],
        payload,
        2048,
        None,
    );
    let dest = temp_dir("traversal-dest");
    let options = recv_options(server.addr(), &dest, 2048);
    let err = recv(&options, &mut |_| {}).expect_err("must reject");
    assert!(matches!(err, Error::PathEscape(_)), "got {err:?}");
    let escaped = dest.parent().unwrap_or(Path::new(".")).join("evil.bin");
    assert!(!escaped.exists(), "不得写出目标目录之外");
    let _ = std::fs::remove_dir_all(&dest);
}

#[test]
fn corrupted_chunk_is_refetched() {
    let payload = pattern(4096);
    let server = FakeServer::start(
        vec![FileEntry {
            path: "data.bin".to_owned(),
            size: payload.len() as u64,
        }],
        payload.clone(),
        4096,
        Some((0, 0)),
    );
    let dest = temp_dir("corrupt-dest");
    let mut options = recv_options(server.addr(), &dest, 4096);
    options.streams = 1;
    let outcome = recv(&options, &mut |_| {}).expect("recv");

    assert_eq!(read_file(&outcome.root.join("data.bin")), payload);
    let requests = server.requests();
    let repeated = requests.iter().filter(|(_, offset)| *offset == 0).count();
    assert!(repeated >= 2, "坏块必须被重新请求，实际 {requests:?}");
    let _ = std::fs::remove_dir_all(&dest);
}

/// 极简源端，用来验证协议防护与坏块重取
struct FakeServer {
    addr: SocketAddr,
    log: Arc<Mutex<Vec<(u32, u64)>>>,
    handle: Option<JoinHandle<()>>,
}

impl FakeServer {
    /// 启动假源端，`corrupt` 指定首次返回坏数据的文件与偏移
    fn start(
        files: Vec<FileEntry>,
        payload: Vec<u8>,
        chunk_bytes: u32,
        corrupt: Option<(u32, u64)>,
    ) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", free_port())).expect("bind");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let addr = listener.local_addr().expect("addr");
        let log = Arc::new(Mutex::new(Vec::new()));
        let server_log = Arc::clone(&log);
        let handle = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut served = 0;
            while served < 2 && Instant::now() < deadline {
                match listener.accept() {
                    Ok((stream, _)) => {
                        // 监听套接字是非阻塞的，accept 出来的连接在 Windows 上会继承该模式
                        stream.set_nonblocking(false).expect("blocking stream");
                        served += 1;
                        let _ = serve_fake_connection(
                            stream,
                            &files,
                            &payload,
                            chunk_bytes,
                            corrupt,
                            &server_log,
                        );
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            addr,
            log,
            handle: Some(handle),
        }
    }

    /// 监听地址
    fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// 已收到的分块请求
    fn requests(&self) -> Vec<(u32, u64)> {
        match self.log.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// 处理一条假源端连接
fn serve_fake_connection(
    stream: TcpStream,
    files: &[FileEntry],
    payload: &[u8],
    chunk_bytes: u32,
    corrupt: Option<(u32, u64)>,
    log: &Mutex<Vec<(u32, u64)>>,
) -> Result<()> {
    let mut reader = std::io::BufReader::new(stream.try_clone().map_err(io)?);
    let mut writer = std::io::BufWriter::new(stream);
    let Some(frame) = read_frame(&mut reader)? else {
        return Ok(());
    };
    let need_manifest = matches!(
        protocol::decode(&frame.payload)?,
        Message::Hello {
            need_manifest: true,
            ..
        }
    );
    let reply = if need_manifest {
        Message::Manifest {
            title: "示例游戏".to_owned(),
            platform: "generic".to_owned(),
            root_name: "demo".to_owned(),
            files: files.to_vec(),
            total_bytes: files.iter().map(|file| file.size).sum(),
            claim_files: Vec::new(),
        }
    } else {
        Message::Ready
    };
    write_frame(&mut writer, &Frame::control(protocol::encode(&reply)?))?;

    let mut corrupted_once = false;
    while let Some(frame) = read_frame(&mut reader)? {
        let Message::RequestChunk { file, offset, len } = protocol::decode(&frame.payload)? else {
            break;
        };
        {
            let mut guard = match log.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.push((file, offset));
        }
        let start = usize::try_from(offset).unwrap_or(0);
        let length = usize::try_from(len).unwrap_or(0);
        let end = start.saturating_add(length).min(payload.len());
        let mut data = payload.get(start..end).unwrap_or_default().to_vec();
        // 摘要按正确内容算，负载改坏，模拟传输途中的静默损坏
        let digest = blake3::hash(&data);
        if let Some((target_file, target_offset)) = corrupt {
            if !corrupted_once && file == target_file && offset == target_offset {
                if let Some(first) = data.first_mut() {
                    *first ^= 0xff;
                }
                corrupted_once = true;
            }
        }
        let mut payload_frame = Vec::with_capacity(32 + data.len());
        payload_frame.extend_from_slice(digest.as_bytes());
        payload_frame.extend_from_slice(&data);
        write_frame(&mut writer, &Frame::chunk(payload_frame))?;
    }
    let _ = chunk_bytes;
    Ok(())
}

/// IO 错误转 crate 错误，直接作为 `map_err` 的处理函数使用
#[allow(clippy::needless_pass_by_value)]
fn io(err: std::io::Error) -> Error {
    Error::Io(err.to_string())
}

#[test]
fn protocol_version_mismatch_is_rejected() {
    let listener = TcpListener::bind(("127.0.0.1", free_port())).expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        let mut reader = std::io::BufReader::new(stream.try_clone().expect("clone"));
        let mut writer = std::io::BufWriter::new(stream);
        if read_frame(&mut reader).ok().flatten().is_none() {
            return;
        }
        let message = Message::Rejected {
            kind: protocol::RejectKind::Version,
            message: "协议版本不一致".to_owned(),
        };
        let _ = write_frame(
            &mut writer,
            &Frame::control(protocol::encode(&message).unwrap_or_default()),
        );
    });

    let dest = temp_dir("version-dest");
    let mut options = recv_options(addr, &dest, 4096);
    options.pairing = None;
    let err = recv(&options, &mut |_| {}).expect_err("must reject");
    assert!(matches!(err, Error::Protocol(_)), "got {err:?}");
    let _ = handle.join();
    let _ = std::fs::remove_dir_all(&dest);
}

/// 不可压缩的确定性数据，避免重复模式干扰分块切点
fn pseudo_random(len: usize) -> Vec<u8> {
    let mut state = 0x0f1e_2d3c_4b5a_6978u64;
    (0..len)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            u8::try_from((state >> 33) % 256).unwrap_or(0)
        })
        .collect()
}

#[test]
fn small_update_transfers_under_ten_percent() {
    let src = temp_dir("update-src");
    let chunk = 64 * 1024u32;
    let v1 = pseudo_random(4 * 1024 * 1024);
    let mut v2 = v1.clone();
    let start = v2.len() / 2;
    let span = v2.len() / 25;
    for (offset, byte) in v2[start..start + span].iter_mut().enumerate() {
        *byte = u8::try_from((offset * 31) % 256).unwrap_or(0) ^ 0x5a;
    }
    std::fs::write(src.join("big.bin"), &v1).expect("write v1");

    let mut host = Host::start(host_options(&src, free_port(), chunk)).expect("host");
    let dest = temp_dir("update-dest");

    // 第一轮全量拉取旧版本
    let options = recv_options(host.local_addr(), &dest, chunk);
    let first = recv(&options, &mut |_| {}).expect("first recv");
    assert_eq!(read_file(&first.root.join("big.bin")), v1);

    // 源端换成新版本，大小不变，清单依旧有效
    std::fs::write(src.join("big.bin"), &v2).expect("write v2");
    let before = host.bytes_sent();

    // 第二轮对已有副本做差异传输
    let mut diff_options = recv_options(host.local_addr(), &dest, chunk);
    diff_options.force = true;
    diff_options.old_root = Some(first.root.clone());
    let second = recv(&diff_options, &mut |_| {}).expect("diff recv");

    let transferred = host.bytes_sent().saturating_sub(before);
    let total = u64::try_from(v2.len()).unwrap_or(0);
    println!(
        "小更新差异传输：改动 {:.1}% 内容，实际传输 {:.1}%",
        span as f64 / total as f64 * 100.0,
        transferred as f64 / total as f64 * 100.0
    );
    assert_eq!(
        read_file(&second.root.join("big.bin")),
        v2,
        "差异传输结果必须与源端一致"
    );
    assert!(
        transferred * 10 < total,
        "改动 4% 时传输量应低于全量 10%，实际 {transferred}/{total}"
    );
    assert!(
        second.bytes_received * 10 < total,
        "会话内接收量也应低于全量 10%"
    );
    host.shutdown();
    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&dest);
}

/// 回环吞吐基准，手动运行：
/// `cargo test -p gamelift-core --test net_transfer -- --ignored --nocapture`
#[test]
#[ignore = "基准用例，需要手动运行并观察输出"]
#[allow(clippy::cast_precision_loss)]
fn loopback_throughput_benchmark() {
    let src = temp_dir("bench-src");
    let payload = pattern(32 * 1024 * 1024);
    std::fs::write(src.join("big.bin"), &payload).expect("write");
    let mut host = Host::start(host_options(&src, free_port(), 8 * 1024 * 1024)).expect("host");
    let dest = temp_dir("bench-dest");
    let mut options = recv_options(host.local_addr(), &dest, 8 * 1024 * 1024);
    options.streams = 4;

    let started = Instant::now();
    let outcome = recv(&options, &mut |_| {}).expect("recv");
    let elapsed = started.elapsed();
    let mib = outcome.bytes_received as f64 / (1024.0 * 1024.0);
    let rate = mib / elapsed.as_secs_f64();
    println!(
        "回环基准: {mib:.1} MiB / {} 流 / {:.2} 秒 = {rate:.1} MiB/s",
        options.streams,
        elapsed.as_secs_f64()
    );

    assert_eq!(read_file(&outcome.root.join("big.bin")), payload);
    host.shutdown();
    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&dest);
}
