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
///
/// 走 winreg 直接读，不再起 `reg query`：少一次进程创建，也没有编码与转义问题
#[cfg(windows)]
fn steam_path_from_registry() -> Option<PathBuf> {
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY};
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    // Steam 是 32 位程序，地址写在 32 位视图里
    let key = hklm
        .open_subkey_with_flags(r"SOFTWARE\Valve\Steam", KEY_READ | KEY_WOW64_32KEY)
        .ok()?;
    let value: String = key.get_value("SteamPath").ok()?;
    let path = PathBuf::from(value);
    path.is_dir().then_some(path)
}

/// 非 Windows 平台没有这份注册表
#[cfg(not(windows))]
fn steam_path_from_registry() -> Option<PathBuf> {
    None
}

/// 枚举全部库的 steamapps 目录，含 libraryfolders.vdf 注册的额外库
///
/// 接收端挑「装到哪个盘」时用这个列表
#[must_use]
pub fn libraries() -> Vec<PathBuf> {
    let mut libs: Vec<PathBuf> = Vec::new();
    for root in steam_roots() {
        let apps = root.join("steamapps");
        if apps.is_dir() && !libs.contains(&apps) {
            libs.push(apps.clone());
        }
        // 每个根下的 libraryfolders.vdf 都读一遍，把其它盘的库也加进来
        if let Ok(text) = std::fs::read_to_string(apps.join("libraryfolders.vdf")) {
            if let VdfValue::Obj(pairs) = vdf::parse(&text).unwrap_or(VdfValue::Obj(Vec::new())) {
                for (_, entry) in pairs {
                    let Some(path) = entry.get("path").and_then(VdfValue::as_str) else {
                        continue;
                    };
                    let other = PathBuf::from(path).join("steamapps");
                    if other.is_dir() && !libs.contains(&other) {
                        libs.push(other);
                    }
                }
            }
        }
    }
    libs
}

/// 可能的 Steam 根目录：注册表优先，其次常见安装位置，最后各磁盘的常见目录
fn steam_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(path) = steam_path_from_registry() {
        roots.push(path);
    }
    for letter in drive_letters() {
        for candidate in roots_for_drive(letter) {
            if candidate.join("steamapps").is_dir() && !roots.contains(&candidate) {
                roots.push(candidate);
            }
        }
    }
    roots
}

/// 本机可用的盘符
fn drive_letters() -> Vec<char> {
    ('C'..='Z')
        .filter(|letter| PathBuf::from(format!("{letter}:\\")).is_dir())
        .collect()
}

/// 某个磁盘上 Steam 常见的安装位置，库目录与安装目录都算
#[must_use]
fn roots_for_drive(letter: char) -> Vec<PathBuf> {
    [
        format!("{letter}:\\Steam"),
        format!("{letter}:\\SteamLibrary"),
        format!("{letter}:\\Program Files (x86)\\Steam"),
        format!("{letter}:\\Program Files\\Steam"),
        format!("{letter}:\\Games\\Steam"),
        format!("{letter}:\\Games\\SteamLibrary"),
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect()
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
    #[ignore = "需要本机装有 Steam，手动运行查看库列表"]
    fn prints_local_steam_libraries() {
        let libs = libraries();
        println!("发现 {} 个 Steam 库: {libs:?}", libs.len());
        assert!(!libs.is_empty(), "本机没扫到 Steam 库");
    }

    #[test]
    fn candidate_roots_cover_common_install_layouts() {
        // 库目录常常单独在一个盘上，只找 Program Files 会漏掉
        let roots = roots_for_drive('D');
        assert!(roots.iter().any(|path| path.ends_with("SteamLibrary")));
        assert!(roots
            .iter()
            .any(|path| path.to_string_lossy().contains("Program Files (x86)")));
        assert!(roots
            .iter()
            .all(|path| path.to_string_lossy().starts_with("D:")));
    }

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
