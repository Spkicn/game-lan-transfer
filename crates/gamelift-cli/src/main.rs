//! `GameLift` CLI：扫描游戏库、配置直连、共享与拉取游戏
//!
//! 用法：
//! ```text
//! gamelift scan                     # 扫描本地游戏库
//! gamelift nics                     # 列出网口，排查直连问题用
//! gamelift link [--host 1|2]        # 配置直连地址，1 为台式机端，2 为笔记本端
//! gamelift unlink                   # 还原直连配置
//! gamelift serve <appid>            # 源端：共享指定 Steam 游戏，需管理员
//! gamelift stop                     # 源端：删除共享，需管理员
//! gamelift pull <appid> --peer IP   # 目标端：拉取游戏并认领
//! ```

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use gamelift_core::{link, transfer};
use gamelift_launchers::adapters;
use gamelift_launchers::steam::SteamAdapter;
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
         \x20 gamelift serve <appid>       源端：共享该 Steam 游戏（需管理员）\n\
         \x20 gamelift stop                源端：删除共享（需管理员）\n\
         \x20 gamelift pull <appid> --peer <IP> [--drive D] [--dest <路径>]\n\
         \x20                              目标端：拉取游戏并给出认领指引"
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
