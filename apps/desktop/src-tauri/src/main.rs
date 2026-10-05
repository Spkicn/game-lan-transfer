//! `GameLift` 桌面应用：`Tauri` 命令胶水层
//!
//! 文件系统、网络与进程操作全部在 Rust 侧完成，前端只负责渲染与状态
//! 每个命令只做参数转换与调用，业务逻辑留在 `gamelift-core` 与 `gamelift-launchers`

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(clippy::needless_pass_by_value)]

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use gamelift_core::net::client::{self, RecvOptions};
use gamelift_core::net::discovery::{self, Peer};
use gamelift_core::net::paths::{join_within, sanitize_relative};
use gamelift_core::net::protocol::ClaimFile;
use gamelift_core::net::server::{self, HostOptions};
use gamelift_core::net::{self, DEFAULT_STREAMS};
use gamelift_core::{link, Error};
use gamelift_launchers::adapters;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

/// 传输进度事件名
const EVENT_PROGRESS: &str = "transfer://progress";

/// 空间预检余量
const SPACE_MARGIN: u64 = 512 * 1024 * 1024;

/// 扫描到的游戏条目
#[derive(Debug, Clone, Serialize)]
struct GameEntry {
    /// 稳定标识，平台加安装目录
    id: String,
    /// 展示名
    name: String,
    /// 平台标识
    platform: String,
    /// 安装目录
    install_dir: String,
    /// 字节数
    size_bytes: u64,
    /// 版本指纹
    fingerprint: Option<String>,
}

/// 发现到的对端
#[derive(Debug, Clone, Serialize)]
struct PeerEntry {
    /// 主机名
    name: String,
    /// 地址
    addr: String,
    /// 会话端口，`0` 表示只接收
    session_port: u16,
    /// 平台标识
    platform: String,
    /// 可用空间
    free_bytes: u64,
}

/// 传输前预检结果
#[derive(Debug, Clone, Serialize)]
struct Preview {
    /// 源端标题
    title: String,
    /// 目标根目录名
    root_name: String,
    /// 内容总字节数
    total_bytes: u64,
    /// 文件数量
    file_count: usize,
    /// 目标盘可用空间
    free_bytes: u64,
    /// 连同余量需要的空间
    need_bytes: u64,
    /// 空间是否够用
    enough: bool,
    /// 目标机上是否已有旧副本，有则启用差异传输
    has_old_copy: bool,
    /// 空间不足时的可执行建议
    advice: String,
}

/// 源端启动结果
#[derive(Debug, Clone, Serialize)]
struct HostInfo {
    /// 监听地址
    addr: String,
    /// 监听端口
    port: u16,
    /// 配对码
    code: String,
    /// 内容标题
    title: String,
}

/// 接收结果
#[derive(Debug, Clone, Serialize)]
struct RecvSummary {
    /// 内容最终目录
    root: String,
    /// 本次会话从网络收到的字节数
    bytes_received: u64,
    /// 内容总字节数
    bytes_total: u64,
    /// 续传或差异跳过的字节数
    bytes_resumed: u64,
    /// 写出的认领文件
    claim_files: Vec<String>,
}

/// 进度事件负载
#[derive(Debug, Clone, Serialize)]
struct ProgressEvent {
    /// 已完成字节
    bytes_done: u64,
    /// 总字节
    bytes_total: u64,
    /// 当前速率
    bytes_per_sec: u64,
    /// 完成比例
    fraction: f64,
}

/// 正在运行的源端
struct RunningHost {
    /// 源端服务
    inner: Mutex<server::Host>,
    /// 广播停止标志
    stop: Arc<AtomicBool>,
}

/// 应用共享状态
#[derive(Default)]
struct AppState {
    /// 当前源端
    host: Mutex<Option<RunningHost>>,
    /// 当前接收任务的取消标志
    cancel: Mutex<Option<Arc<AtomicBool>>>,
}

/// 扫描本机已安装的游戏
#[tauri::command]
fn scan_games() -> Vec<GameEntry> {
    let mut games = Vec::new();
    for adapter in adapters() {
        let platform = adapter.platform().to_owned();
        let Ok(found) = adapter.scan() else {
            continue;
        };
        for game in found {
            games.push(GameEntry {
                id: format!("{}:{}", platform, game.install_dir.display()),
                name: game.display_name,
                platform: platform.clone(),
                install_dir: game.install_dir.to_string_lossy().into_owned(),
                size_bytes: game.size_bytes,
                fingerprint: game.version_fingerprint,
            });
        }
    }
    games.sort_by(|left, right| left.name.cmp(&right.name));
    games
}

/// 在直连网段里发现对端
///
/// # Errors
///
/// 本机地址不可解析或发现端口被占用时返回说明
#[tauri::command]
fn discover_peers(iface: Option<String>, seconds: u64) -> Result<Vec<PeerEntry>, String> {
    let ip = local_ip(iface.as_deref())?;
    let socket = discovery::bind(ip, discovery::DISCOVERY_PORT).map_err(describe)?;
    let timeout = Duration::from_secs(seconds.clamp(1, 30));
    let peers = discovery::collect(&socket, timeout, None).map_err(describe)?;
    Ok(peers
        .into_iter()
        .map(|peer| PeerEntry {
            name: peer.name,
            addr: peer.addr.to_string(),
            session_port: peer.session_port,
            platform: peer.platform,
            free_bytes: peer.free_bytes,
        })
        .collect())
}

/// 传输预检：取对端清单、目标盘空间与是否可差异传输
///
/// # Errors
///
/// 连接失败、目标目录不可写或空间查询失败时返回说明
#[tauri::command]
fn preview_target(
    peer: String,
    want: String,
    dest: String,
    pairing: Option<String>,
) -> Result<Preview, String> {
    let addr = parse_peer(&peer)?;
    let info = client::probe(addr, pairing.as_deref(), &want).map_err(describe)?;
    let dest_parent = PathBuf::from(&dest);
    std::fs::create_dir_all(&dest_parent).map_err(io_text)?;
    let free_bytes = link::free_bytes_at(&dest_parent).map_err(describe)?;
    let has_old_copy = existing_root(&dest_parent, &info.root_name)?.is_some();
    let need_bytes = info.total_bytes.saturating_add(SPACE_MARGIN);
    let enough = free_bytes >= need_bytes;
    let advice = if enough {
        String::new()
    } else {
        format!(
            "目标盘还差 {}，先清理该盘上的旧游戏或换一个盘再传",
            human_size(need_bytes.saturating_sub(free_bytes))
        )
    };
    Ok(Preview {
        title: info.title,
        root_name: info.root_name,
        total_bytes: info.total_bytes,
        file_count: info.file_count,
        free_bytes,
        need_bytes,
        enough,
        has_old_copy,
        advice,
    })
}

/// 启动源端服务并开始广播
///
/// # Errors
///
/// 目录不存在、端口占用或广播套接字创建失败时返回说明
#[tauri::command]
fn start_host(
    state: State<'_, AppState>,
    install_dir: String,
    title: String,
    platform: String,
    code: Option<String>,
    iface: Option<String>,
) -> Result<HostInfo, String> {
    stop_running_host(&state);
    let ip = local_ip(iface.as_deref())?;
    let root = PathBuf::from(&install_dir);
    if !root.is_dir() {
        return Err(format!("目录不存在: {}", root.display()));
    }
    let pairing = code
        .filter(|value| !value.is_empty())
        .unwrap_or_else(discovery::new_pairing_code);
    let root_name = root.file_name().map_or_else(
        || "transfer".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let host = server::Host::start(HostOptions {
        root: root.clone(),
        extra_roots: Vec::new(),
        bind: SocketAddr::new(ip, net::DEFAULT_SESSION_PORT),
        pairing: Some(pairing.clone()),
        title: title.clone(),
        platform: platform.clone(),
        root_name: Some(root_name),
        claim_files: claim_files_for(&platform, &root),
        chunk_bytes: net::DEFAULT_CHUNK_BYTES,
    })
    .map_err(describe)?;
    let addr = host.local_addr();
    let stop = Arc::new(AtomicBool::new(false));
    let announce = Peer {
        instance: discovery::new_instance(),
        name: discovery::local_name(),
        addr: ip,
        session_port: addr.port(),
        platform,
        free_bytes: link::free_bytes_at(&root).unwrap_or(0),
        pairing_required: true,
    };
    spawn_announcer(ip, announce, Arc::clone(&stop))?;
    *lock(&state.host) = Some(RunningHost {
        inner: Mutex::new(host),
        stop,
    });
    Ok(HostInfo {
        addr: ip.to_string(),
        port: addr.port(),
        code: pairing,
        title,
    })
}

/// 停止源端服务与广播
#[tauri::command]
fn stop_host(state: State<'_, AppState>) {
    stop_running_host(&state);
}

/// 停掉当前源端并等待广播线程退出
fn stop_running_host(state: &AppState) {
    if let Some(running) = lock(&state.host).take() {
        running.stop.store(true, Ordering::Relaxed);
        if let Ok(mut host) = running.inner.lock() {
            host.shutdown();
        }
    }
}

/// 接收内容，进度通过 `transfer://progress` 事件上报
///
/// # Errors
///
/// 握手被拒、空间不足或传输失败时返回说明
#[allow(clippy::too_many_arguments)]
#[tauri::command]
fn start_recv(
    app: AppHandle,
    state: State<'_, AppState>,
    peer: String,
    want: String,
    dest: String,
    platform: Option<String>,
    pairing: Option<String>,
    force: bool,
) -> Result<RecvSummary, String> {
    let addr = parse_peer(&peer)?;
    let dest_parent = PathBuf::from(&dest);
    std::fs::create_dir_all(&dest_parent).map_err(io_text)?;
    let info = client::probe(addr, pairing.as_deref(), &want).map_err(describe)?;
    let old_root = existing_root(&dest_parent, &info.root_name)?;
    let cancel = Arc::new(AtomicBool::new(false));
    *lock(&state.cancel) = Some(Arc::clone(&cancel));
    let options = RecvOptions {
        peer: addr,
        pairing,
        want,
        dest_parent,
        claim_root: claim_root_for(platform.as_deref().unwrap_or("steam"), &info.root_name),
        streams: DEFAULT_STREAMS,
        chunk_bytes: net::DEFAULT_CHUNK_BYTES,
        cancel: Some(Arc::clone(&cancel)),
        force,
        old_root,
        layout: client::Layout::Wrapper,
    };
    let mut last = Instant::now();
    let outcome = client::recv(&options, &mut |stats| {
        if last.elapsed() >= Duration::from_millis(300) {
            last = Instant::now();
            let _ = app.emit(
                EVENT_PROGRESS,
                ProgressEvent {
                    bytes_done: stats.bytes_done,
                    bytes_total: stats.bytes_total,
                    bytes_per_sec: stats.bytes_per_sec,
                    fraction: stats.fraction(),
                },
            );
        }
    })
    .map_err(describe)?;
    *lock(&state.cancel) = None;
    Ok(RecvSummary {
        root: outcome.root.to_string_lossy().into_owned(),
        bytes_received: outcome.bytes_received,
        bytes_total: outcome.bytes_total,
        bytes_resumed: outcome.bytes_resumed,
        claim_files: outcome
            .claim_files
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
    })
}

/// 取消正在进行的接收，已完成的块会保留
#[tauri::command]
fn cancel_recv(state: State<'_, AppState>) {
    if let Some(flag) = lock(&state.cancel).as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
}

/// 本机默认目标目录：Steam 库的 `common` 目录
#[tauri::command]
fn default_dest() -> Option<String> {
    let root = gamelift_launchers::steam::steam_root()?;
    Some(
        root.join("steamapps")
            .join("common")
            .to_string_lossy()
            .into_owned(),
    )
}

/// 网卡条目
#[derive(Debug, Clone, Serialize)]
struct NicEntry {
    /// 连接名
    name: String,
    /// 硬件描述
    description: String,
    /// 是否为物理以太网口
    is_physical: bool,
    /// 当前 IPv4
    ip: Option<String>,
}

/// 网络现状，供界面展示与选地址
#[derive(Debug, Clone, Serialize)]
struct NetworkStatus {
    /// 全部网卡
    nics: Vec<NicEntry>,
    /// 将要使用的本机地址
    address: Option<String>,
    /// 该地址所在网卡名
    nic_name: Option<String>,
    /// 该网卡是否为物理以太网口
    is_physical: bool,
    /// 是否已落在直连网段
    direct_link: bool,
    /// 下一步提示
    hint: String,
}

/// 读取网卡与可用地址
#[tauri::command]
fn network_status() -> NetworkStatus {
    let nics = link::list_nics();
    let picked = link::preferred_ipv4(&nics);
    let (address, nic_name, is_physical, direct_link) = match &picked {
        Some(found) => (
            Some(found.ip.clone()),
            Some(found.nic_name.clone()),
            found.is_physical,
            link::is_direct_link_ip(&found.ip),
        ),
        None => (None, None, false, false),
    };
    let hint = if address.is_none() {
        "没有可用的本机地址：先连上网络，或点「配置直连」自动配一个".to_owned()
    } else if direct_link {
        "已配好直连地址，插上网线就能与对端互通".to_owned()
    } else if is_physical {
        "用的是以太网口，建议点「配置直连」把两台机器放进同一网段".to_owned()
    } else {
        "当前走无线网卡，同一局域网内也能传；插网线会快很多".to_owned()
    };
    NetworkStatus {
        nics: nics
            .into_iter()
            .map(|nic| NicEntry {
                name: nic.name,
                description: nic.description,
                is_physical: nic.is_physical,
                ip: nic.current_ipv4,
            })
            .collect(),
        address,
        nic_name,
        is_physical,
        direct_link,
        hint,
    }
}

/// 给物理以太网口配置直连地址，需要管理员权限
///
/// # Errors
///
/// 找不到物理网口或提权失败时返回说明
#[tauri::command]
fn setup_link(host: u8) -> Result<String, String> {
    link::setup_direct_link(host).map_err(describe)
}

/// 还原直连配置，两端都清理一遍
///
/// # Errors
///
/// 还原失败时返回说明
#[tauri::command]
fn revert_link() -> Result<(), String> {
    link::revert_direct_link(1).map_err(describe)?;
    let _ = link::revert_direct_link(2);
    Ok(())
}

fn main() {
    let result = tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            scan_games,
            discover_peers,
            preview_target,
            start_host,
            stop_host,
            start_recv,
            cancel_recv,
            default_dest,
            network_status,
            setup_link,
            revert_link
        ])
        .run(tauri::generate_context!());
    if let Err(err) = result {
        eprintln!("启动 GameLift 失败: {err}");
        std::process::exit(1);
    }
}

/// 解析本机地址，未给定时优先物理以太网口，其次任意有地址的网卡
///
/// # Errors
///
/// 本机没有可用地址或地址不可解析时返回说明
fn local_ip(explicit: Option<&str>) -> Result<IpAddr, String> {
    if let Some(text) = explicit.filter(|value| !value.is_empty()) {
        return text
            .parse()
            .map_err(|_| format!("地址不合法: {text}，示例 192.168.88.1"));
    }
    let picked = link::preferred_ipv4(&link::list_nics())
        .ok_or_else(|| "没有可用的本机地址，请先连上网络，或在界面里点「配置直连」".to_owned())?;
    picked
        .ip
        .parse()
        .map_err(|_| format!("网口地址不合法: {}", picked.ip))
}

/// 解析对端地址，接受 `IP` 或 `IP:端口`
///
/// # Errors
///
/// 地址不可解析时返回说明
fn parse_peer(peer: &str) -> Result<SocketAddr, String> {
    if let Ok(addr) = peer.parse::<SocketAddr>() {
        return Ok(addr);
    }
    let ip: IpAddr = peer
        .parse()
        .map_err(|_| format!("对端地址不合法: {peer}，示例 192.168.88.2"))?;
    Ok(SocketAddr::new(ip, net::DEFAULT_SESSION_PORT))
}

/// 目标机上已有的同内容副本路径
///
/// # Errors
///
/// 目标根目录名非法时返回说明
fn existing_root(dest_parent: &Path, root_name: &str) -> Result<Option<PathBuf>, String> {
    let relative = sanitize_relative(root_name).map_err(describe)?;
    if relative.components().count() != 1 {
        return Ok(None);
    }
    let candidate = join_within(dest_parent, &relative).map_err(describe)?;
    Ok(candidate.is_dir().then_some(candidate))
}

/// 按平台推断认领文件根目录
fn claim_root_for(platform: &str, root_name: &str) -> Option<PathBuf> {
    match platform {
        "steam" => {
            let _ = root_name;
            gamelift_launchers::steam::steam_root().map(|root| root.join("steamapps"))
        }
        "epic" => gamelift_launchers::epic::manifests_dir(),
        _ => None,
    }
}

/// 按平台收集认领文件
fn claim_files_for(platform: &str, install_dir: &Path) -> Vec<ClaimFile> {
    match platform {
        "epic" => gamelift_launchers::epic::claim_files_for(install_dir),
        "steam" => steam_claim_files(install_dir),
        _ => Vec::new(),
    }
}

/// 找到与该安装目录匹配的 `appmanifest`，作为 Steam 的认领文件
fn steam_claim_files(install_dir: &Path) -> Vec<ClaimFile> {
    let Some(steamapps) = install_dir.parent().and_then(Path::parent) else {
        return Vec::new();
    };
    let Some(game_dir_name) = install_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
    else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(steamapps) else {
        return Vec::new();
    };
    let needle = format!("\"{game_dir_name}\"");
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        let is_acf = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("acf"));
        if !lower.starts_with("appmanifest_") || !is_acf {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if content.contains(&needle) {
            return vec![ClaimFile {
                relative_path: name,
                content,
            }];
        }
    }
    Vec::new()
}

/// 后台每秒广播一次自身信息，直到收到停止标志
///
/// # Errors
///
/// 广播套接字创建或线程启动失败时返回说明
fn spawn_announcer(ip: IpAddr, peer: Peer, stop: Arc<AtomicBool>) -> Result<(), String> {
    let socket = discovery::bind(ip, 0).map_err(describe)?;
    let target = broadcast_target(ip);
    thread::Builder::new()
        .name("gamelift-announce".to_owned())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let _ = discovery::announce_to(&socket, target, &peer);
                thread::sleep(Duration::from_secs(1));
            }
        })
        .map_err(io_text)?;
    Ok(())
}

/// 广播目标地址
fn broadcast_target(ip: IpAddr) -> SocketAddr {
    match ip {
        IpAddr::V4(v4) => SocketAddr::new(
            IpAddr::V4(discovery::directed_broadcast(v4)),
            discovery::DISCOVERY_PORT,
        ),
        IpAddr::V6(_) => SocketAddr::new(ip, discovery::DISCOVERY_PORT),
    }
}

/// 取互斥锁，锁中毒时继续使用内部值
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// 把 crate 错误转成前端可读的说明
fn describe(err: Error) -> String {
    err.to_string()
}

/// 把 IO 错误转成前端可读的说明
fn io_text(err: std::io::Error) -> String {
    err.to_string()
}

/// 人类可读的容量
#[allow(clippy::cast_precision_loss)]
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}
