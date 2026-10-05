//! `Steam` 适配器
//!
//! 扫描 `steamapps` 下的 `appmanifest_*.acf`，版本指纹取 `buildid` 与各 `InstalledDepots[].manifest`

use std::path::PathBuf;

use crate::{AdapterError, InstalledGame, LauncherAdapter};

/// Steam 平台适配器。
pub struct SteamAdapter;

impl LauncherAdapter for SteamAdapter {
    fn platform(&self) -> &'static str {
        "steam"
    }

    fn scan(&self) -> Result<Vec<InstalledGame>, AdapterError> {
        if let Some(steamapps) = steamapps_dir() {
            scan_library(&steamapps)
        } else {
            Err(AdapterError::LauncherNotFound {
                platform: "steam".into(),
            })
        }
    }
}

/// 探测默认 Steam 库目录，未安装时返回 None
/// 后续改为注册表级探测，并解析 libraryfolders.vdf 的多库路径
fn steamapps_dir() -> Option<PathBuf> {
    let candidates = [
        r"D:\Program Files (x86)\Steam\steamapps",
        r"C:\Program Files (x86)\Steam\steamapps",
    ];
    candidates.iter().map(PathBuf::from).find(|p| p.is_dir())
}

/// 扫描一个 steamapps 目录下的 appmanifest_*.acf，当前仅按文件是否存在登记
/// 后续解析 acf 内容获取名称、SizeOnDisk 与 buildid
fn scan_library(steamapps: &std::path::Path) -> Result<Vec<InstalledGame>, AdapterError> {
    let mut games = Vec::new();
    let entries = std::fs::read_dir(steamapps).map_err(|e| AdapterError::ParseFailed {
        platform: "steam".into(),
        reason: format!("无法读取库目录: {e}"),
    })?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("appmanifest_") && name.ends_with(".acf") {
            games.push(InstalledGame {
                display_name: name
                    .trim_start_matches("appmanifest_")
                    .trim_end_matches(".acf")
                    .to_owned(),
                install_dir: steamapps.join("common"),
                size_bytes: 0,
                version_fingerprint: None,
            });
        }
    }
    Ok(games)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn scan_missing_dir_is_parse_failed() {
        let err = scan_library(PathBuf::from("Z:/definitely/not/here").as_path())
            .expect_err("missing dir must fail");
        assert!(
            matches!(err, AdapterError::ParseFailed { .. }),
            "got: {err:?}"
        );
    }
}
