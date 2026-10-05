//! GOG Galaxy 适配器
//!
//! GOG 把安装记录写在注册表 `HKLM\SOFTWARE\WOW6432Node\GOG.com\Games\<id>`，
//! 值里有 `gameName`、`path` 与 `buildId`；安装目录里另有 `goggame-<id>.info`

use std::path::{Path, PathBuf};

use crate::registry::{RegistryRead, WindowsRegistry};
use crate::{dir_name, AdapterError, InstalledGame, LauncherAdapter};

/// GOG 安装记录的注册表根
pub const ROOT: &str = "HKLM\\SOFTWARE\\WOW6432Node\\GOG.com\\Games";

/// GOG Galaxy 适配器
#[derive(Debug, Clone, Copy, Default)]
pub struct GogAdapter;

impl GogAdapter {
    /// 从给定的注册表读取
    ///
    /// # Errors
    ///
    /// 注册表里没有任何 GOG 记录时返回 [`AdapterError::LauncherNotFound`]
    pub fn scan_from(registry: &impl RegistryRead) -> Result<Vec<InstalledGame>, AdapterError> {
        let ids = registry.subkeys(ROOT);
        if ids.is_empty() {
            return Err(AdapterError::LauncherNotFound {
                platform: "gog".to_owned(),
            });
        }
        let mut out = Vec::new();
        for id in ids {
            let values = registry.values(&format!("{ROOT}\\{id}"));
            let Some(dir) = values.get("path").filter(|value| !value.is_empty()) else {
                continue;
            };
            let install_dir = PathBuf::from(dir);
            if !install_dir.is_dir() {
                continue;
            }
            let info = read_game_info(&install_dir, &id);
            let display_name = values
                .get("gameName")
                .filter(|value| !value.is_empty())
                .cloned()
                .or_else(|| info.as_ref().and_then(|entry| entry.0.clone()))
                .unwrap_or_else(|| dir_name(&install_dir));
            let fingerprint = values
                .get("buildId")
                .filter(|value| !value.is_empty())
                .cloned()
                .or_else(|| info.and_then(|entry| entry.1));
            out.push(InstalledGame {
                display_name,
                install_dir,
                size_bytes: 0,
                version_fingerprint: fingerprint,
            });
        }
        Ok(out)
    }
}

impl LauncherAdapter for GogAdapter {
    fn platform(&self) -> &'static str {
        "gog"
    }

    fn scan(&self) -> Result<Vec<InstalledGame>, AdapterError> {
        Self::scan_from(&WindowsRegistry)
    }
}

/// 从安装目录里的 `goggame-<id>.info` 取名字与 buildId
fn read_game_info(install_dir: &Path, id: &str) -> Option<(Option<String>, Option<String>)> {
    let path = install_dir.join(format!("goggame-{id}.info"));
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let name = value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let build = value.get("buildId").and_then(|build| match build {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        _ => None,
    });
    Some((name, build))
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use crate::registry::{MapRegistry, ValueMap};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gamelift-gog-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn missing_registry_reports_launcher_not_found() {
        let registry = MapRegistry::default();
        let err = GogAdapter::scan_from(&registry).expect_err("must fail");
        assert!(matches!(err, AdapterError::LauncherNotFound { .. }));
    }

    #[test]
    fn reads_name_and_fingerprint_from_registry() {
        let install = temp_dir("registry");
        let registry = MapRegistry::new([(
            format!("{ROOT}\\1207658691"),
            ValueMap::from([
                ("gameName".to_owned(), "Example Quest".to_owned()),
                ("path".to_owned(), install.to_string_lossy().into_owned()),
                ("buildId".to_owned(), "54321".to_owned()),
            ]),
        )]);
        let games = GogAdapter::scan_from(&registry).expect("scan");
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].display_name, "Example Quest");
        assert_eq!(games[0].install_dir, install);
        assert_eq!(games[0].version_fingerprint.as_deref(), Some("54321"));
        let _ = std::fs::remove_dir_all(&install);
    }

    #[test]
    fn falls_back_to_info_file_and_skips_missing_paths() {
        let install = temp_dir("info");
        std::fs::write(
            install.join("goggame-42.info"),
            r#"{"gameId":"42","name":"从信息文件读到的名字","buildId":98765}"#,
        )
        .expect("write");
        let registry = MapRegistry::new([
            (
                format!("{ROOT}\\42"),
                ValueMap::from([("path".to_owned(), install.to_string_lossy().into_owned())]),
            ),
            (
                format!("{ROOT}\\43"),
                ValueMap::from([("path".to_owned(), "Z:\\not-there".to_owned())]),
            ),
        ]);
        let games = GogAdapter::scan_from(&registry).expect("scan");
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].display_name, "从信息文件读到的名字");
        assert_eq!(games[0].version_fingerprint.as_deref(), Some("98765"));
        let _ = std::fs::remove_dir_all(&install);
    }
}
