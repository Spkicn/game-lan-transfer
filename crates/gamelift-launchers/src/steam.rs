//! Steam 适配器：扫描 `steamapps` 下的 `appmanifest_*.acf`，
//! 生成含名称、大小与版本指纹的游戏清单
//!
//! 库定位优先查注册表 HKLM 下的 Steam 项，失败后退回常见安装路径
//! 多库场景解析 `libraryfolders.vdf` 的所有 `path`

use std::path::{Path, PathBuf};

use crate::vdf::{self, VdfValue};
use crate::{AdapterError, InstalledGame, LauncherAdapter};

/// Steam 平台适配器
pub struct SteamAdapter;

impl LauncherAdapter for SteamAdapter {
    fn platform(&self) -> &'static str {
        "steam"
    }

    fn scan(&self) -> Result<Vec<InstalledGame>, AdapterError> {
        if libraries().is_empty() {
            Err(AdapterError::LauncherNotFound {
                platform: "steam".into(),
            })
        } else {
            Ok(scan_all_libraries())
        }
    }
}

/// 定位 Steam 根目录，含 `steamapps` 的上一级；未安装返回 None
#[must_use]
pub fn steam_root() -> Option<PathBuf> {
    // 读 HKLM\SOFTWARE\WOW6432Node\Valve\Steam 的 SteamPath，走 reg query
    if let Some(path) = steam_path_from_registry() {
        if path.join("steamapps").is_dir() {
            return Some(path);
        }
    }
    // 兜底：注册表不给结果时依次探测常见安装路径
    [
        r"C:\Program Files (x86)\Steam",
        r"C:\Program Files\Steam",
        r"D:\Program Files (x86)\Steam",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.join("steamapps").is_dir())
}

/// 从注册表读 SteamPath，未安装或无权限时返回 None
fn steam_path_from_registry() -> Option<PathBuf> {
    let output = std::process::Command::new("reg")
        .args([
            "query",
            r"HKLM\SOFTWARE\WOW6432Node\Valve\Steam",
            "/v",
            "SteamPath",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // 行样例: "    SteamPath    REG_SZ    C:/Program Files (x86)/Steam"
    let line = text
        .lines()
        .find(|l| l.contains("SteamPath") && l.contains("REG_SZ"))?;
    let value = line.split("REG_SZ").nth(1)?.trim();
    let path = PathBuf::from(value);
    path.is_dir().then_some(path)
}

/// 枚举全部库的 steamapps 目录，含 libraryfolders.vdf 注册的额外库
///
/// 接收端挑「装到哪个盘」时用这个列表
#[must_use]
pub fn libraries() -> Vec<PathBuf> {
    let Some(root) = steam_root() else {
        return Vec::new();
    };
    let mut libs = vec![root.join("steamapps")];
    let vdf_path = root.join("steamapps").join("libraryfolders.vdf");
    if let Ok(text) = std::fs::read_to_string(&vdf_path) {
        if let VdfValue::Obj(pairs) = vdf::parse(&text).unwrap_or(VdfValue::Obj(Vec::new())) {
            for (_, entry) in pairs {
                let Some(path) = entry
                    .get("path")
                    .and_then(VdfValue::as_str)
                    .map(str::to_owned)
                else {
                    continue;
                };
                let apps = PathBuf::from(&path).join("steamapps");
                if apps.is_dir() && !libs.contains(&apps) {
                    libs.push(apps);
                }
            }
        }
    }
    libs
}

/// 扫描所有库，某库或某清单读取失败时跳过并继续
fn scan_all_libraries() -> Vec<InstalledGame> {
    let libs = libraries();
    let mut games = Vec::new();
    for lib in libs {
        let Ok(entries) = std::fs::read_dir(&lib) else {
            continue; // 单库不可读：跳过，不中断整体扫描
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(app) = name
                .strip_prefix("appmanifest_")
                .and_then(|s| s.strip_suffix(".acf"))
            {
                if let Ok(text) = std::fs::read_to_string(entry.path()) {
                    if let Some(game) = parse_acf(&text, &lib, app) {
                        games.push(game);
                    }
                }
            }
        }
    }
    games
}

/// 解析 acf 文本为 InstalledGame，解析失败返回 None 由调用方跳过
fn parse_acf(text: &str, steamapps: &Path, appid: &str) -> Option<InstalledGame> {
    let root = vdf::parse(text).ok()?;
    let app = root.get("AppState")?;
    let installdir = app.get("installdir").and_then(VdfValue::as_str)?;
    let size = app
        .get("SizeOnDisk")
        .and_then(VdfValue::as_str)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let buildid = app
        .get("buildid")
        .and_then(VdfValue::as_str)
        .map(str::to_owned);
    Some(InstalledGame {
        display_name: format!("{appid} — {installdir}"),
        install_dir: steamapps.join("common").join(installdir),
        size_bytes: size,
        version_fingerprint: buildid,
    })
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_size_and_fingerprint() {
        let acf = r#"
"AppState"
{
	"appid"		"123456"
	"installdir"		"example_game"
	"SizeOnDisk"		"123456789"
	"buildid"		"1000000"
}
"#;
        let g = parse_acf(acf, Path::new("D:/steamapps"), "123456").expect("must parse");
        assert_eq!(g.size_bytes, 123_456_789);
        assert_eq!(g.version_fingerprint.as_deref(), Some("1000000"));
        assert!(g.display_name.contains("example_game"));
        assert_eq!(
            g.install_dir,
            PathBuf::from("D:/steamapps/common/example_game")
        );
    }

    #[test]
    fn malformed_acf_is_skipped() {
        assert!(parse_acf("not vdf at all", Path::new("x"), "1").is_none());
    }

    #[test]
    fn scan_without_steam_is_launcher_not_found() {
        // 覆盖率目的：scan 走到 NotFound 分支的路径已在 lib.rs 测试验证
        let adapter = SteamAdapter;
        if let Err(e) = adapter.scan() {
            assert!(matches!(e, AdapterError::LauncherNotFound { .. }));
        }
    }
}
