//! Ubisoft Connect 适配器
//!
//! Ubisoft 把安装记录写在注册表
//! `HKCU\SOFTWARE\Ubisoft\Launcher\Installs\<id>`，值里有 `InstallDir`；
//! 只写了 `InstallState` 的条目表示还没真正落盘，扫描时跳过

use std::path::PathBuf;

use crate::registry::{RegistryRead, WindowsRegistry};
use crate::{dir_name, AdapterError, InstalledGame, LauncherAdapter};

/// Ubisoft 安装记录的注册表根
pub const ROOT: &str = "HKCU\\SOFTWARE\\Ubisoft\\Launcher\\Installs";

/// Ubisoft Connect 适配器
#[derive(Debug, Clone, Copy, Default)]
pub struct UbisoftAdapter;

impl UbisoftAdapter {
    /// 从给定的注册表读取
    ///
    /// # Errors
    ///
    /// 注册表里没有 Ubisoft 记录时返回 [`AdapterError::LauncherNotFound`]
    pub fn scan_from(registry: &impl RegistryRead) -> Result<Vec<InstalledGame>, AdapterError> {
        let ids = registry.subkeys(ROOT);
        if ids.is_empty() {
            return Err(AdapterError::LauncherNotFound {
                platform: "ubisoft".to_owned(),
            });
        }
        let mut out = Vec::new();
        for id in ids {
            let values = registry.values(&format!("{ROOT}\\{id}"));
            let Some(dir) = values
                .get("InstallDir")
                .or_else(|| values.get("Install Dir"))
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let install_dir = PathBuf::from(dir);
            if !install_dir.is_dir() {
                continue;
            }
            out.push(InstalledGame {
                display_name: values
                    .get("DisplayName")
                    .filter(|value| !value.is_empty())
                    .cloned()
                    .unwrap_or_else(|| dir_name(&install_dir)),
                install_dir,
                size_bytes: 0,
                version_fingerprint: values.get("InstallState").cloned(),
            });
        }
        Ok(out)
    }
}

impl LauncherAdapter for UbisoftAdapter {
    fn platform(&self) -> &'static str {
        "ubisoft"
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
        let dir = std::env::temp_dir().join(format!("gamelift-ubi-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn missing_registry_reports_launcher_not_found() {
        let err = UbisoftAdapter::scan_from(&MapRegistry::default()).expect_err("must fail");
        assert!(matches!(err, AdapterError::LauncherNotFound { .. }));
    }

    #[test]
    fn only_entries_with_install_dir_become_games() {
        let install = temp_dir("installed");
        let registry = MapRegistry::new([
            (
                format!("{ROOT}\\7080"),
                ValueMap::from([
                    (
                        "InstallDir".to_owned(),
                        install.to_string_lossy().into_owned(),
                    ),
                    ("InstallState".to_owned(), "1".to_owned()),
                ]),
            ),
            (
                format!("{ROOT}\\7081"),
                ValueMap::from([("InstallState".to_owned(), "0x1".to_owned())]),
            ),
        ]);
        let games = UbisoftAdapter::scan_from(&registry).expect("scan");
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].install_dir, install);
        assert_eq!(games[0].display_name, dir_name(&install));
        assert_eq!(games[0].version_fingerprint.as_deref(), Some("1"));
        let _ = std::fs::remove_dir_all(&install);
    }

    #[test]
    fn skips_entries_pointing_at_missing_directories() {
        let registry = MapRegistry::new([(
            format!("{ROOT}\\9000"),
            ValueMap::from([("InstallDir".to_owned(), "Z:\\gone".to_owned())]),
        )]);
        let games = UbisoftAdapter::scan_from(&registry).expect("scan");
        assert_eq!(games, Vec::<InstalledGame>::new());
    }
}
