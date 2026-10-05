//! `Epic` 平台适配器：解析 Epic Games Launcher 的 `.item` 清单
//!
//! 清单目录默认取 `%ProgramData%\Epic\EpicGamesLauncher\Data\Manifests`
//! 每份 `.item` 是一个 `JSON` 对象，含展示名、安装目录与版本标识

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{AdapterError, InstalledGame, LauncherAdapter};

/// Epic 清单在 `ProgramData` 下的相对目录
pub const MANIFEST_RELATIVE: &str = "Epic/EpicGamesLauncher/Data/Manifests";

/// `Epic` 平台适配器
pub struct EpicAdapter;

impl LauncherAdapter for EpicAdapter {
    fn platform(&self) -> &'static str {
        "epic"
    }

    fn scan(&self) -> Result<Vec<InstalledGame>, AdapterError> {
        let not_found = AdapterError::LauncherNotFound {
            platform: "epic".to_owned(),
        };
        let Some(dir) = manifests_dir() else {
            return Err(not_found);
        };
        if !dir.is_dir() {
            return Err(not_found);
        }
        scan_from(&dir)
    }
}

/// 默认清单目录，取不到 `ProgramData` 时返回 `None`
#[must_use]
pub fn manifests_dir() -> Option<PathBuf> {
    std::env::var_os("PROGRAMDATA").map(|root| PathBuf::from(root).join(MANIFEST_RELATIVE))
}

/// `.item` 清单里用得到的字段
#[derive(Debug, Deserialize)]
struct ItemManifest {
    /// 展示名
    #[serde(rename = "DisplayName")]
    display_name: Option<String>,
    /// 安装目录
    #[serde(rename = "InstallLocation")]
    install_location: Option<String>,
    /// 应用标识
    #[serde(rename = "AppName")]
    app_name: Option<String>,
    /// 应用版本
    #[serde(rename = "AppVersion")]
    app_version: Option<String>,
    /// 构建标识，版本指纹优先用它
    #[serde(rename = "BuildLabel")]
    build_label: Option<String>,
    /// 占用字节数
    #[serde(rename = "InstallSize")]
    install_size: Option<u64>,
}

/// 读取一份清单
fn read_manifest(path: &Path) -> Option<ItemManifest> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<ItemManifest>(&text).ok()
}

/// 扫描指定清单目录，坏清单与已卸载残留直接跳过
///
/// # Errors
///
/// 目录不可读时返回 [`AdapterError::ParseFailed`]
pub fn scan_from(dir: &Path) -> Result<Vec<InstalledGame>, AdapterError> {
    let entries = std::fs::read_dir(dir).map_err(|err| AdapterError::ParseFailed {
        platform: "epic".to_owned(),
        reason: format!("读取 {} 失败: {err}", dir.display()),
    })?;
    let mut games = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("item") {
            continue;
        }
        let Some(manifest) = read_manifest(&path) else {
            continue;
        };
        let Some(location) = manifest.install_location else {
            continue;
        };
        let install_dir = PathBuf::from(location.replace('\\', "/"));
        if !install_dir.is_dir() {
            continue;
        }
        let display_name = manifest
            .display_name
            .or(manifest.app_name)
            .unwrap_or_else(|| {
                install_dir.file_name().map_or_else(
                    || "Epic 游戏".to_owned(),
                    |name| name.to_string_lossy().into_owned(),
                )
            });
        games.push(InstalledGame {
            display_name,
            install_dir,
            size_bytes: manifest.install_size.unwrap_or(0),
            version_fingerprint: manifest.build_label.or(manifest.app_version),
        });
    }
    games.sort_by(|left, right| left.display_name.cmp(&right.display_name));
    Ok(games)
}

/// 认领文件：把该游戏的 `.item` 清单原样带到目标机的清单目录
///
/// 找不到对应清单或读取失败时返回空列表，调用方按"无认领文件"处理
#[must_use]
pub fn claim_files_for(install_dir: &Path) -> Vec<gamelift_core::net::protocol::ClaimFile> {
    let Some(dir) = manifests_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let wanted = install_dir.to_string_lossy().replace('\\', "/");
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("item") {
            continue;
        }
        let Some(manifest) = read_manifest(&path) else {
            continue;
        };
        let Some(location) = manifest.install_location else {
            continue;
        };
        if !location.replace('\\', "/").eq_ignore_ascii_case(&wanted) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(name) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        return vec![gamelift_core::net::protocol::ClaimFile {
            relative_path: name,
            content,
        }];
    }
    Vec::new()
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gamelift-epic-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn write_item(dir: &Path, name: &str, json: &str) {
        std::fs::write(dir.join(name), json).expect("write");
    }

    /// 清单里的路径统一用正斜杠，与 Epic 实际写出的格式一致
    fn json_path(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    fn item_json(display: &str, install: &str, build: &str) -> String {
        format!(
            r#"{{"DisplayName":"{display}","InstallLocation":"{install}","AppName":"{display}","BuildLabel":"{build}","InstallSize":123456}}"#
        )
    }

    #[test]
    fn scans_installed_games() {
        let root = temp_dir("scan");
        let manifests = root.join("manifests");
        let install_a = root.join("GameA");
        let install_b = root.join("GameB");
        std::fs::create_dir_all(&manifests).expect("mkdir");
        std::fs::create_dir_all(&install_a).expect("mkdir");
        std::fs::create_dir_all(&install_b).expect("mkdir");
        write_item(
            &manifests,
            "A.item",
            &item_json("Game A", &json_path(&install_a), "build-1"),
        );
        write_item(
            &manifests,
            "B.item",
            &item_json("Game B", &json_path(&install_b), "build-2"),
        );
        write_item(&manifests, "broken.item", "{ not json");
        write_item(&manifests, "notes.txt", "ignored");
        write_item(
            &manifests,
            "gone.item",
            &item_json("Gone", &json_path(&root.join("Missing")), "build-3"),
        );

        let games = scan_from(&manifests).expect("scan");
        let names: Vec<&str> = games
            .iter()
            .map(|game| game.display_name.as_str())
            .collect();
        assert_eq!(names, vec!["Game A", "Game B"]);
        assert_eq!(games[0].size_bytes, 123_456);
        assert_eq!(games[0].version_fingerprint.as_deref(), Some("build-1"));
        assert_eq!(games[1].install_dir, install_b);
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn missing_directory_is_parse_failed() {
        let err = scan_from(Path::new("Z:/definitely/not/here")).expect_err("must fail");
        assert!(
            matches!(err, AdapterError::ParseFailed { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn build_label_wins_over_app_version() {
        let root = temp_dir("fingerprint");
        let manifests = root.join("manifests");
        let install = root.join("GameC");
        std::fs::create_dir_all(&manifests).expect("mkdir");
        std::fs::create_dir_all(&install).expect("mkdir");
        write_item(
            &manifests,
            "C.item",
            &format!(
                r#"{{"DisplayName":"Game C","InstallLocation":"{}","AppVersion":"1.2.3","BuildLabel":"build-9"}}"#,
                json_path(&install)
            ),
        );
        let games = scan_from(&manifests).expect("scan");
        assert_eq!(games[0].version_fingerprint.as_deref(), Some("build-9"));
        std::fs::remove_dir_all(&root).expect("cleanup");
    }
}
