//! 战网适配器
//!
//! 战网把每个已安装游戏记在注册表
//! `HKLM\SOFTWARE\WOW6432Node\Blizzard Entertainment\<产品名>`，
//! 值里有 `InstallLocation`；安装目录里的 `.build.info` 记着版本号

use std::path::{Path, PathBuf};

use crate::registry::{RegistryRead, WindowsRegistry};
use crate::{dir_name, AdapterError, InstalledGame, LauncherAdapter};

/// 战网安装记录的注册表根
pub const ROOT: &str = "HKLM\\SOFTWARE\\WOW6432Node\\Blizzard Entertainment";

/// 这些子键是启动器自身，不是游戏
const NOT_A_GAME: &[&str] = &["Battle.net", "Blizzard Browser", "Blizzard Entertainment"];

/// 战网适配器
#[derive(Debug, Clone, Copy, Default)]
pub struct BattleNetAdapter;

impl BattleNetAdapter {
    /// 从给定的注册表读取
    ///
    /// # Errors
    ///
    /// 注册表里没有战网记录时返回 [`AdapterError::LauncherNotFound`]
    pub fn scan_from(registry: &impl RegistryRead) -> Result<Vec<InstalledGame>, AdapterError> {
        let products = registry.subkeys(ROOT);
        if products.is_empty() {
            return Err(AdapterError::LauncherNotFound {
                platform: "battlenet".to_owned(),
            });
        }
        let mut out = Vec::new();
        for product in products {
            if NOT_A_GAME.contains(&product.as_str()) {
                continue;
            }
            let values = registry.values(&format!("{ROOT}\\{product}"));
            let Some(dir) = values
                .get("InstallLocation")
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let install_dir = PathBuf::from(dir);
            if !install_dir.is_dir() {
                continue;
            }
            let fingerprint = read_build_info(&install_dir);
            out.push(InstalledGame {
                display_name: product,
                install_dir,
                size_bytes: 0,
                version_fingerprint: fingerprint,
            });
        }
        Ok(out)
    }
}

impl LauncherAdapter for BattleNetAdapter {
    fn platform(&self) -> &'static str {
        "battlenet"
    }

    fn scan(&self) -> Result<Vec<InstalledGame>, AdapterError> {
        Self::scan_from(&WindowsRegistry)
    }
}

/// `.build.info` 第一行是竖线分隔的字段，第一个字段是 build key
fn read_build_info(install_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(install_dir.join(".build.info")).ok()?;
    let first = text.lines().next()?;
    let key = first.split('|').next()?.trim();
    (!key.is_empty()).then(|| key.to_owned())
}

/// 产品目录名，注册表里的键名偶尔为空时兜底
#[allow(dead_code)]
fn fallback_name(install_dir: &Path) -> String {
    dir_name(install_dir)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use crate::registry::{MapRegistry, ValueMap};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gamelift-bnet-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn missing_registry_reports_launcher_not_found() {
        let err = BattleNetAdapter::scan_from(&MapRegistry::default()).expect_err("must fail");
        assert!(matches!(err, AdapterError::LauncherNotFound { .. }));
    }

    #[test]
    fn reads_install_location_and_build_key() {
        let install = temp_dir("build");
        std::fs::write(install.join(".build.info"), "abc123|1|2|us|0|0\n").expect("write");
        let registry = MapRegistry::new([
            (
                format!("{ROOT}\\World of Warcraft"),
                ValueMap::from([(
                    "InstallLocation".to_owned(),
                    install.to_string_lossy().into_owned(),
                )]),
            ),
            (
                format!("{ROOT}\\Battle.net"),
                ValueMap::from([("InstallLocation".to_owned(), "C:\\Launcher".to_owned())]),
            ),
        ]);
        let games = BattleNetAdapter::scan_from(&registry).expect("scan");
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].display_name, "World of Warcraft");
        assert_eq!(games[0].version_fingerprint.as_deref(), Some("abc123"));
        let _ = std::fs::remove_dir_all(&install);
    }

    #[test]
    fn skips_products_without_install_location() {
        let registry = MapRegistry::new([
            (format!("{ROOT}\\Overwatch"), ValueMap::new()),
            (
                format!("{ROOT}\\Diablo IV"),
                ValueMap::from([("InstallLocation".to_owned(), "Z:\\gone".to_owned())]),
            ),
        ]);
        let games = BattleNetAdapter::scan_from(&registry).expect("scan");
        assert_eq!(games, Vec::<InstalledGame>::new());
    }
}
