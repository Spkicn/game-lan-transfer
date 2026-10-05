//! `GameLift` CLI：扫描游戏库、配置直连、搬运游戏
//!
//! 用法：
//! ```text
//! gamelift scan                     # 扫描本地游戏库
//! gamelift nics                     # 列出网口，排查直连问题用
//! gamelift link [--host 1|2]        # 配置直连地址，1 为台式机端，2 为笔记本端
//! gamelift unlink                   # 还原直连配置
//! gamelift peers [--iface IP]       # 发现直连网段里的对端
//! gamelift host <appid> [--dir 路径] [--code NNNNNN]
//!                                   # 源端：启动分块传输服务，无需管理员
//! gamelift recv <appid> --peer IP [--code NNNNNN] [--dest 路径]
//!                                   # 目标端：接收并认领
//! gamelift serve <appid>            # 源端：共享指定 Steam 游戏，需管理员，SMB 兜底
//! gamelift stop                     # 源端：删除共享，需管理员
//! gamelift pull <appid> --peer IP   # 目标端：走 SMB 拉取，需管理员，SMB 兜底
//! ```

use std::io::Write as _;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use gamelift_core::net::client::{self, RecvOptions};
use gamelift_core::net::discovery::{self, Peer};
use gamelift_core::net::paths::{join_within, sanitize_relative};
use gamelift_core::net::protocol::ClaimFile;
use gamelift_core::net::server::{self, HostOptions};
use gamelift_core::net::{self, DEFAULT_STREAMS};
use gamelift_core::{link, transfer, TransferStats};
use gamelift_launchers::adapters;
use gamelift_launchers::steam::{self, SteamAdapter};
use gamelift_launchers::LauncherAdapter as _;

fn main() {
    if let Err(err) = run() {
        eprintln!("错误: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("scan") => scan(),
        Some("nics") => nics(),
        Some("link") => link_cmd(args.get(1).map(String::as_str)),
        Some("unlink") => unlink_cmd(),
        Some("serve") => serve_cmd(args.get(1).map(String::as_str)),
        Some("stop") => stop_cmd(),
        Some("pull") => pull_cmd(&args[1..]),
        Some("peers") => peers_cmd(&args[1..]),
        Some("host") => host_cmd(&args[1..]),
        Some("recv") => recv_cmd(&args[1..]),
        Some("help") | None => {
            print_usage();
            Ok(())
        }
        Some(other) => {
            print_usage();
            bail!("未知命令: {other}");
        }
    }
}

fn print_usage() {
    println!(
        "GameLift —— 游戏局域网直传\n\
         \n\
         用法:\n\
         \x20 gamelift scan                扫描本地游戏库\n\
         \x20 gamelift nics                列出网口\n\
         \x20 gamelift link [--host 1|2]   配置直连地址（需管理员）\n\
         \x20 gamelift unlink              还原直连配置（需管理员）\n\
         \x20 gamelift peers [--seconds N] 发现直连网段里的对端\n\
         \x20 gamelift host <appid> [--dir <路径>] [--code NNNNNN] [--iface <本机IP>]\n\
         \x20                              源端：启动分块传输服务\n\
         \x20 gamelift recv <appid> --peer <IP> [--code NNNNNN] [--dest <路径>] [--streams N] [--force]\n\
         \x20                              目标端：接收并认领\n\
         \x20 gamelift serve <appid>       源端：共享该 Steam 游戏（需管理员，SMB 兜底）\n\
         \x20 gamelift stop                源端：删除共享（需管理员）\n\
         \x20 gamelift pull <appid> --peer <IP> [--drive D] [--dest <路径>]\n\
         \x20                              目标端：走 SMB 拉取并给出认领指引"
    );
}

fn scan() -> Result<()> {
    for adapter in adapters() {
        match adapter.scan() {
            Ok(games) => {
                println!("[{}] {} 个游戏", adapter.platform(), games.len());
                for game in games {
                    println!(
                        "  {:<12} {:>10}  {}",
                        game.version_fingerprint.as_deref().unwrap_or("-"),
                        human_size(game.size_bytes),
                        game.display_name
                    );
                }
            }
            Err(gamelift_launchers::AdapterError::LauncherNotFound { platform }) => {
                println!("[{platform}] 未安装，跳过");
            }
            Err(err) => return Err(err.into()),
        }
    }
    println!("\n提示: serve/pull 用 appid（display_name 中 “ — ” 前的部分）指定游戏");
    Ok(())
}

fn nics() -> Result<()> {
    let nics = link::list_nics();
    if nics.is_empty() {
        bail!("未获取到网口信息（Get-NetAdapter 不可用）");
    }
    println!("{:<24} {:<8} {:<16} 描述", "名称", "物理口", "当前IP");
    for nic in nics {
        println!(
            "{:<24} {:<8} {:<16} {}",
            nic.name,
            if nic.is_physical { "是" } else { "否" },
            nic.current_ipv4.as_deref().unwrap_or("-"),
            nic.description
        );
    }
    Ok(())
}

fn link_cmd(host: Option<&str>) -> Result<()> {
    // 默认 1 为台式机端，笔记本端传 2
    let octet: u8 = match host {
        None | Some("--host" | "1") => 1,
        Some("2") => 2,
        Some(other) => bail!("--host 只接受 1（台式机）或 2（笔记本），收到: {other}"),
    };
    let ip = link::setup_direct_link(octet).context("配置直连地址失败")?;
    println!("直连地址已就绪: {ip}（无网关，不影响 WLAN 默认路由）");
    println!(
        "对端地址: {}（对端用 --host {} 配另一端）",
        link::peer_ip(3 - octet),
        3 - octet
    );
    Ok(())
}

fn unlink_cmd() -> Result<()> {
    link::revert_direct_link(1).context("还原直连配置失败")?;
    link::revert_direct_link(2).ok(); // 顺带清理另一端的约定地址，未配置时静默跳过
    println!("直连配置已还原（DHCP 恢复，无残留地址）");
    Ok(())
}

fn serve_cmd(appid: Option<&str>) -> Result<()> {
    let Some(appid) = appid else {
        bail!("用法: gamelift serve <appid>")
    };
    let games = SteamAdapter.scan().map_err(|e| anyhow::anyhow!("{e}"))?;
    let game = games
        .iter()
        .find(|g| g.display_name.starts_with(&format!("{appid} —")))
        .with_context(|| format!("未找到 appid {appid}，先运行 gamelift scan"))?;
    let steamapps = game
        .install_dir
        .parent()
        .and_then(|common| common.parent())
        .map(PathBuf::from)
        .with_context(|| format!("无法定位 steamapps 目录: {}", game.install_dir.display()))?;
    transfer::serve(&steamapps).context("创建共享失败")?;
    println!(
        "已共享 {} → \\\\{{本机IP}}\\{}（对端 gamelift pull {appid} --peer <本机IP>）",
        steamapps.display(),
        transfer::SHARE_NAME
    );
    println!("传完记得运行 gamelift stop 清理共享");
    Ok(())
}

fn stop_cmd() -> Result<()> {
    transfer::stop_serve().context("删除共享失败")?;
    println!("共享已删除，系统零残留");
    Ok(())
}

fn pull_cmd(args: &[String]) -> Result<()> {
    let appid = args.first().map(String::as_str).with_context(|| {
        "用法: gamelift pull <appid> --peer <IP> [--drive D] [--dest <路径>]".to_owned()
    })?;
    let mut peer = None;
    let mut drive = "D".to_owned();
    let mut dest: Option<PathBuf> = None;
    let mut iter = args[1..].iter();
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--peer" => {
                peer = iter.next().cloned();
            }
            "--drive" => {
                if let Some(d) = iter.next() {
                    drive.clone_from(d);
                }
            }
            "--dest" => {
                dest = iter.next().map(PathBuf::from);
            }
            other => bail!("pull 不认识参数: {other}"),
        }
    }
    let peer = peer.with_context(|| "缺少 --peer（对端 IP，如 192.168.88.2）".to_owned())?;

    // 从对端拉 appmanifest 需要知道 installdir：本地清单已有该 appid 时复用其 installdir，
    // 否则要求 --dest 显式指定目标目录
    let (remote_relative, dest, estimated) = resolve_pull_target(appid, dest.as_deref())?;

    let free = link::free_bytes(&drive).context("查询目标盘可用空间失败")?;
    println!(
        "拉取 {appid}（约 {}）← {peer}，目标盘 {drive}: 剩余 {}",
        human_size(estimated),
        human_size(free)
    );
    let copied =
        transfer::pull(&peer, &remote_relative, &dest, &drive, estimated).context("拉取失败")?;
    println!(
        "已复制约 {}（robocopy /Z 支持断点续传）",
        human_size(copied)
    );
    println!();
    println!(
        "{}",
        transfer::claim_instructions(appid, remote_relative.as_str())
    );
    Ok(())
}

/// 确定 pull 的远端相对路径与本地目标目录
fn resolve_pull_target(
    appid: &str,
    dest: Option<&std::path::Path>,
) -> Result<(String, PathBuf, u64)> {
    // 本地已安装同一游戏时直接复用其 installdir 与大小
    if let Ok(games) = SteamAdapter.scan() {
        if let Some(game) = games
            .iter()
            .find(|g| g.display_name.starts_with(&format!("{appid} —")))
        {
            let installdir = game
                .install_dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .with_context(|| format!("无法确定 installdir: {}", game.install_dir.display()))?;
            let dest = dest.map_or_else(|| game.install_dir.clone(), PathBuf::from);
            return Ok((installdir, dest, game.size_bytes));
        }
    }
    // 未安装时要求传 --dest 指定目标目录
    let Some(dest) = dest else {
        bail!(
            "本地未安装 appid {appid}，无法推断 installdir。\n\
             两种解法:\n\
             \x20 1. 传 --dest 指定目标目录（ Steam 侧为 <steamapps>\\common\\<游戏目录> ）\n\
             \x20 2. 在源端运行 gamelift scan 查看 installdir 后填入"
        );
    };
    Ok((".".to_owned(), dest.to_path_buf(), 0))
}

/// 发现直连网段里的对端
fn peers_cmd(args: &[String]) -> Result<()> {
    let seconds = parse_flag(args, "--seconds")?.unwrap_or(5);
    let local_ip = resolve_iface(flag_value(args, "--iface"))?;
    let socket =
        discovery::bind(local_ip, discovery::DISCOVERY_PORT).context("绑定发现端口失败")?;
    println!("在 {local_ip} 上监听发现广播，最多 {seconds} 秒…");
    let peers =
        discovery::collect(&socket, Duration::from_secs(seconds), None).context("收集公告失败")?;
    if peers.is_empty() {
        println!("未发现对端。请确认对端已运行 gamelift host，且两端在同一网段。");
        return Ok(());
    }
    println!(
        "{:<20} {:<28} {:<10} {:>12}  配对",
        "名称", "地址", "平台", "可用空间"
    );
    for peer in peers {
        let address = if peer.session_port == 0 {
            format!("{}（仅接收）", peer.addr)
        } else {
            format!("{}:{}", peer.addr, peer.session_port)
        };
        let pairing = if peer.pairing_required {
            "需要"
        } else {
            "不需要"
        };
        println!(
            "{:<20} {:<28} {:<10} {:>12}  {pairing}",
            peer.name,
            address,
            peer.platform,
            human_size(peer.free_bytes)
        );
    }
    Ok(())
}

/// 源端：启动分块传输服务并持续广播
fn host_cmd(args: &[String]) -> Result<()> {
    let target = args
        .first()
        .filter(|arg| !arg.starts_with("--"))
        .cloned()
        .context("用法: gamelift host <appid> [--dir <路径>] [--code NNNNNN] [--iface <本机IP>]")?;
    let bind_ip = resolve_iface(flag_value(args, "--iface"))?;
    let port = parse_flag(args, "--port")?.unwrap_or(net::DEFAULT_SESSION_PORT);
    let pairing =
        flag_value(args, "--code").map_or_else(discovery::new_pairing_code, str::to_owned);

    let (root, title, platform, root_name, claim_files) = match flag_value(args, "--dir") {
        Some(dir) => {
            let path = PathBuf::from(dir);
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .with_context(|| format!("无法从路径推断目录名: {}", path.display()))?;
            (path, name.clone(), "generic".to_owned(), name, Vec::new())
        }
        None => resolve_steam_source(&target)?,
    };
    let free_bytes = link::free_bytes_at(&root).unwrap_or(0);
    let host = server::Host::start(HostOptions {
        root: root.clone(),
        bind: SocketAddr::new(bind_ip, port),
        pairing: Some(pairing.clone()),
        title: title.clone(),
        platform: platform.clone(),
        root_name: Some(root_name.clone()),
        claim_files,
        chunk_bytes: net::DEFAULT_CHUNK_BYTES,
    })
    .context("启动传输服务失败，检查端口是否被占用")?;

    println!("内容: {title}（目录 {root_name}）");
    println!("监听: {bind_ip}:{}", host.local_addr().port());
    println!("配对码: {pairing}");
    println!("对端执行: gamelift recv {target} --peer {bind_ip} --code {pairing}");
    println!("按 Ctrl+C 停止");

    let socket = discovery::bind(bind_ip, 0).context("创建广播套接字失败")?;
    let peer = Peer {
        instance: discovery::new_instance(),
        name: discovery::local_name(),
        addr: bind_ip,
        session_port: host.local_addr().port(),
        platform,
        free_bytes,
        pairing_required: true,
    };
    let broadcast = broadcast_target(bind_ip);
    let mut warned = false;
    loop {
        if let Err(err) = discovery::announce_to(&socket, broadcast, &peer) {
            if !warned {
                warned = true;
                eprintln!("广播公告失败，对端仍可用 --peer 手动指定地址: {err}");
            }
        }
        thread::sleep(Duration::from_secs(1));
    }
}

/// 目标端：接收内容并写入认领文件
fn recv_cmd(args: &[String]) -> Result<()> {
    let want = args
        .first()
        .filter(|arg| !arg.starts_with("--"))
        .cloned()
        .context(
            "用法: gamelift recv <appid|目录名> --peer <IP> [--code NNNNNN] [--dest <路径>]",
        )?;
    let peer_ip: IpAddr = flag_value(args, "--peer")
        .context("缺少 --peer（源端地址，如 192.168.88.2）")?
        .parse()
        .context("--peer 需要 IP 地址")?;
    let port = parse_flag(args, "--port")?.unwrap_or(net::DEFAULT_SESSION_PORT);
    let streams = parse_flag(args, "--streams")?.unwrap_or(DEFAULT_STREAMS);
    let (dest_parent, claim_root) = resolve_recv_dest(flag_value(args, "--dest"), &want)?;
    let pairing = flag_value(args, "--code").map(str::to_owned);
    let peer = SocketAddr::new(peer_ip, port);
    let info = client::probe(peer, pairing.as_deref(), &want).context("探测源端失败")?;
    let old_root = diff_source(&dest_parent, &info.root_name)?;
    let options = RecvOptions {
        peer,
        pairing,
        want: want.clone(),
        dest_parent: dest_parent.clone(),
        claim_root,
        streams,
        chunk_bytes: net::DEFAULT_CHUNK_BYTES,
        cancel: None,
        force: has_flag(args, "--force"),
        old_root: old_root.clone(),
    };
    println!(
        "从 {peer_ip}:{port} 接收 {want}（{}，{} 个文件），目标 {}",
        human_size(info.total_bytes),
        info.file_count,
        dest_parent.display()
    );
    if let Some(root) = &old_root {
        println!(
            "检测到已有副本 {}，将只传变化的块（覆盖它需要 --force）",
            root.display()
        );
    }
    let announce_stop = match receiver_announcer(&dest_parent, flag_value(args, "--iface")) {
        Ok(stop) => Some(stop),
        Err(err) => {
            eprintln!("提示: 未启动发现广播，不影响本次接收: {err}");
            None
        }
    };
    let mut last: Option<Instant> = None;
    let received = client::recv(&options, &mut |stats| {
        let due = last.is_none_or(|mark| mark.elapsed() >= Duration::from_secs(1));
        if due {
            last = Some(Instant::now());
            print!("\r{}", progress_line(stats));
            let _ = std::io::stdout().flush();
        }
    });
    if let Some(stop) = announce_stop {
        stop.store(true, Ordering::Relaxed);
    }
    let outcome = received.context("接收失败")?;
    println!();
    println!(
        "已接收 {} → {}",
        human_size(outcome.bytes_total),
        outcome.root.display()
    );
    if outcome.bytes_resumed > 0 {
        println!("其中续传跳过 {}", human_size(outcome.bytes_resumed));
    }
    for path in &outcome.claim_files {
        println!("已写入认领文件 {}", path.display());
    }
    if outcome.claim_files.is_empty() {
        println!();
        println!("{}", transfer::claim_instructions(&want, "内容目录"));
    }
    Ok(())
}

/// 定位源端的 Steam 游戏目录与清单文件
fn resolve_steam_source(appid: &str) -> Result<(PathBuf, String, String, String, Vec<ClaimFile>)> {
    let games = SteamAdapter.scan().map_err(|err| anyhow!("{err}"))?;
    let game = games
        .iter()
        .find(|game| game.display_name.starts_with(&format!("{appid} —")))
        .with_context(|| format!("未找到 appid {appid}，先运行 gamelift scan"))?;
    let steamapps = game
        .install_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .with_context(|| format!("无法定位 steamapps 目录: {}", game.install_dir.display()))?;
    let root_name = game
        .install_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .with_context(|| format!("无法确定 installdir: {}", game.install_dir.display()))?;
    let acf_name = transfer::acf_filename(appid);
    let acf_path = steamapps.join(&acf_name);
    let content = std::fs::read_to_string(&acf_path)
        .with_context(|| format!("读取清单失败: {}", acf_path.display()))?;
    let claim_files = vec![ClaimFile {
        relative_path: acf_name,
        content,
    }];
    Ok((
        game.install_dir.clone(),
        game.display_name.clone(),
        "steam".to_owned(),
        root_name,
        claim_files,
    ))
}

/// 目标父目录与认领根目录
fn resolve_recv_dest(dest: Option<&str>, want: &str) -> Result<(PathBuf, Option<PathBuf>)> {
    if let Some(dir) = dest {
        let parent = PathBuf::from(dir);
        std::fs::create_dir_all(&parent)
            .with_context(|| format!("创建目标目录失败: {}", parent.display()))?;
        let claim_root = parent
            .file_name()
            .filter(|name| name.eq_ignore_ascii_case("common"))
            .and_then(|_| parent.parent())
            .map(Path::to_path_buf);
        return Ok((parent, claim_root));
    }
    let steamapps = steam::steam_root()
        .map(|root| root.join("steamapps"))
        .with_context(|| {
            format!("本机未找到 Steam 库，无法推断 {want} 的目标位置，请用 --dest 指定目录")
        })?;
    Ok((steamapps.join("common"), Some(steamapps)))
}

/// 目标机上已有的同内容副本路径，存在时用于差异传输
fn diff_source(dest_parent: &Path, root_name: &str) -> Result<Option<PathBuf>> {
    let relative = sanitize_relative(root_name)?;
    if relative.components().count() != 1 {
        return Ok(None);
    }
    let candidate = join_within(dest_parent, &relative)?;
    Ok(candidate.is_dir().then_some(candidate))
}

/// 解析本机直连地址
fn resolve_iface(explicit: Option<&str>) -> Result<IpAddr> {
    if let Some(text) = explicit {
        return text
            .parse()
            .with_context(|| format!("--iface 需要本机直连地址，收到: {text}"));
    }
    let nic = link::list_nics()
        .into_iter()
        .find(|nic| nic.is_physical)
        .context("找不到物理以太网口，请用 --iface 指定本机地址")?;
    let text = nic.current_ipv4.with_context(|| {
        format!(
            "{} 尚未配置地址，先运行 gamelift link，或用 --iface 指定",
            nic.name
        )
    })?;
    text.parse()
        .with_context(|| format!("无法解析网口地址: {text}"))
}

/// 接收期间广播自己的存在，便于源端用 `gamelift peers` 看到目标机与可用空间
fn receiver_announcer(dest_parent: &Path, iface: Option<&str>) -> Result<Arc<AtomicBool>> {
    let local_ip = resolve_iface(iface)?;
    let socket = discovery::bind(local_ip, 0).context("创建广播套接字失败")?;
    let peer = Peer {
        instance: discovery::new_instance(),
        name: discovery::local_name(),
        addr: local_ip,
        session_port: 0,
        platform: discovery::local_platform(),
        free_bytes: link::free_bytes_at(dest_parent).unwrap_or(0),
        pairing_required: true,
    };
    let broadcast = broadcast_target(local_ip);
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    thread::Builder::new()
        .name("gamelift-announce".to_owned())
        .spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                let _ = discovery::announce_to(&socket, broadcast, &peer);
                thread::sleep(Duration::from_secs(1));
            }
        })
        .context("启动广播线程失败")?;
    Ok(stop)
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

/// 取 `--flag value` 形式参数的值
fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let index = args.iter().position(|arg| arg == flag)?;
    args.get(index + 1).map(String::as_str)
}

/// 是否出现开关形式的参数
fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

/// 解析 `--flag 数字` 形式的参数
fn parse_flag<T>(args: &[String], flag: &str) -> Result<Option<T>>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match flag_value(args, flag) {
        Some(text) => text
            .parse::<T>()
            .map(Some)
            .map_err(|err| anyhow!("{flag} 取值非法: {err}")),
        None => Ok(None),
    }
}

/// 一行进度文本
fn progress_line(stats: &TransferStats) -> String {
    let eta = stats
        .bytes_total
        .saturating_sub(stats.bytes_done)
        .checked_div(stats.bytes_per_sec)
        .map_or_else(|| "计算中".to_owned(), format_duration);
    format!(
        "{:>6.1}%  {} / {}  速度 {}/s  剩余 {eta}    ",
        stats.fraction() * 100.0,
        human_size(stats.bytes_done),
        human_size(stats.bytes_total),
        human_size(stats.bytes_per_sec)
    )
}

/// 人类可读的时长
fn format_duration(seconds: u64) -> String {
    if seconds >= 3600 {
        format!("{} 小时 {} 分", seconds / 3600, (seconds % 3600) / 60)
    } else if seconds >= 60 {
        format!("{} 分 {} 秒", seconds / 60, seconds % 60)
    } else {
        format!("{seconds} 秒")
    }
}

/// 人类可读的容量
/// u64→f64 的精度损失对展示无影响
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
