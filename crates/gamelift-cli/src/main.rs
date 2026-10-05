//! `GameLift` CLI，当前实现 `scan` 子命令

use anyhow::Result;
use gamelift_launchers::adapters;

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
        Some("help") | None => {
            println!("用法: gamelift <scan>");
            Ok(())
        }
        Some(other) => {
            anyhow::bail!("未知命令: {other}（可用: scan）");
        }
    }
}

fn scan() -> Result<()> {
    for adapter in adapters() {
        match adapter.scan() {
            Ok(games) => {
                println!("[{}] {} 个游戏", adapter.platform(), games.len());
                for game in games {
                    println!("  {}", game.display_name);
                }
            }
            Err(gamelift_launchers::AdapterError::LauncherNotFound { platform }) => {
                println!("[{platform}] 未安装，跳过");
            }
            Err(err) => return Err(err.into()),
        }
    }
    Ok(())
}
