//! EA 适配器
//!
//! EA 把已安装游戏记在注册表
//! `HKLM\SOFTWARE\WOW6432Node\Electronic Arts\EA Games\<游戏>`，
//! 值里有 `Install Dir`、`DisplayName` 与 `Product GUID`

use std::path::PathBuf;

use crate::registry::{RegistryRead, WindowsRegistry};
use crate::{AdapterError, InstalledGame, LauncherAdapter};

/// EA 安装记录的注册表根
pub const ROOT: &str = "HKLM\\SOFTWARE\\WOW6432Node\\Electronic Arts\\EA Games";

/// EA 适配器
#[derive(Debug, Clone, Copy, Default)]
pub struct EaAdapter;

impl EaAdapter {
    /// 从给定的注册表读取
    ///
    /// # Errors
    ///
    /// 注册表里没有 EA 记录时返回 [`AdapterError::LauncherNotFound`]
    pub fn scan_from(registry: &impl RegistryRead) -> Result<Vec<InstalledGame>, AdapterError> {
        let entries = registry.subkeys(ROOT);
        if entries.is_empty() {
            return Err(AdapterError::LauncherNotFound {
                platform: "ea".to_owned(),
            });
        }
        let mut out = Vec::new();
        for entry in entries {
            let values = registry.values(&format!("{ROOT}\\{entry}"));
            let Some(dir) = values
                .get("Install Dir")
                .or_else(|| values.get("InstallDir"))
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let install_dir = PathBuf::from(dir);
            if !install_dir.is_dir() {
                continue;
            }
            let display_name = values
                .get("DisplayName")
                .filter(|value| !value.is_empty())
                .cloned()
                .unwrap_or(entry);
            let fingerprint = values
                .get("Product GUID")
                .or_else(|| values.get("Version"))
                .filter(|value| !value.is_empty())
                .cloned();
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

impl LauncherAdapter for EaAdapter {
    fn platform(&self) -> &'static str {
        "ea"
    }

    fn scan(&self) -> Result<Vec<InstalledGame>, AdapterError> {
        Self::scan_from(&WindowsRegistry)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use crate::registry::{MapRegistry, ValueMap};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gamelift-ea-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn missing_registry_reports_launcher_not_found() {
        let err = EaAdapter::scan_from(&MapRegistry::default()).expect_err("must fail");
        assert!(matches!(err, AdapterError::LauncherNotFound { .. }));
    }

    #[test]
    fn reads_display_name_and_guid() {
        let install = temp_dir("guid");
        let registry = MapRegistry::new([(
            format!("{ROOT}\\Demo Game"),
            ValueMap::from([
                (
                    "Install Dir".to_owned(),
                    install.to_string_lossy().into_owned(),
                ),
                ("DisplayName".to_owned(), "示例作品".to_owned()),
                ("Product GUID".to_owned(), "{abc-123}".to_owned()),
            ]),
        )]);
        let games = EaAdapter::scan_from(&registry).expect("scan");
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].display_name, "示例作品");
        assert_eq!(games[0].version_fingerprint.as_deref(), Some("{abc-123}"));
        let _ = std::fs::remove_dir_all(&install);
    }

    #[test]
    fn falls_back_to_key_name_and_skips_missing_dirs() {
        let install = temp_dir("fallback");
        let registry = MapRegistry::new([
            (
                format!("{ROOT}\\NoName"),
                ValueMap::from([(
                    "Install Dir".to_owned(),
                    install.to_string_lossy().into_owned(),
                )]),
            ),
            (
                format!("{ROOT}\\Gone"),
                ValueMap::from([("Install Dir".to_owned(), "Z:\\gone".to_owned())]),
            ),
        ]);
        let games = EaAdapter::scan_from(&registry).expect("scan");
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].display_name, "NoName");
        assert_eq!(games[0].version_fingerprint, None);
        let _ = std::fs::remove_dir_all(&install);
    }
}
