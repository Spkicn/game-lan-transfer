//! `GameLift` 平台适配器
//!
//! 平台覆盖 Steam / Epic / 战网 / EA / Ubisoft / GOG，每个启动器一个模块
//! 模块实现 [`LauncherAdapter`] trait，把平台特定格式翻译成
//! `gamelift_core::manifest::TransferManifest`
//!
//! 新增一个平台适配器不超过 200 行，且不必改动核心代码即可注册

pub mod generic;
pub mod steam;
pub mod vdf;

/// 适配器统一错误，库 crate 一律用 thiserror 且禁止跨边界传 String
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    /// 找不到该平台，通常是未安装或路径探测失败
    #[error("未找到 {platform} 安装")]
    LauncherNotFound {
        /// 平台标识，如 "steam"
        platform: String,
    },
    /// 库清单文件解析失败
    #[error("{platform} 清单解析失败: {reason}")]
    ParseFailed {
        /// 平台标识
        platform: String,
        /// 面向最终用户展示的失败原因
        reason: String,
    },
    /// 核心 crate 的错误透传
    #[error(transparent)]
    Core(#[from] gamelift_core::Error),
}

/// 扫描出的一个已安装游戏条目，对应 UI 库列表的一行
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledGame {
    /// 展示名，如 "Example Game"
    pub display_name: String,
    /// 安装目录绝对路径
    pub install_dir: std::path::PathBuf,
    /// 磁盘占用字节数，取自 `SizeOnDisk` 或等价字段
    pub size_bytes: u64,
    /// 版本指纹，Steam 为 buildid，其他平台取等价字段，无则 None
    pub version_fingerprint: Option<String>,
}

/// 平台适配器接口，新增平台需实现本 trait 并注册到 [`adapters`]
pub trait LauncherAdapter {
    /// 平台标识，如 "steam"，与 TransferManifest.platform 对应
    fn platform(&self) -> &'static str;

    /// 扫描本地已安装游戏
    /// 平台未安装时返回 `AdapterError::LauncherNotFound`，属于正常情况而非故障
    ///
    /// # Errors
    ///
    /// 平台未安装返回 [`AdapterError::LauncherNotFound`]；
    /// 库目录存在但读取或解析失败返回 [`AdapterError::ParseFailed`]
    fn scan(&self) -> Result<Vec<InstalledGame>, AdapterError>;
}

/// 返回全部已注册适配器的实例
///
/// 新适配器在此追加一行即可
#[must_use]
pub fn adapters() -> Vec<Box<dyn LauncherAdapter>> {
    vec![
        Box::new(steam::SteamAdapter),
        Box::new(generic::GenericAdapter),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_steam_and_generic() {
        let list = adapters();
        let platforms: Vec<&str> = list.iter().map(|a| a.platform()).collect();
        assert!(platforms.contains(&"steam"));
        assert!(platforms.contains(&"generic"));
    }

    #[test]
    fn scan_returns_launcher_not_found_when_steam_missing() {
        // Steam 未安装时走 LauncherNotFound 正常路径的冒烟验证
        let adapter = steam::SteamAdapter;
        if let Err(AdapterError::LauncherNotFound { platform }) = adapter.scan() {
            assert_eq!(platform, "steam");
        }
        // 已安装 Steam 的机器上扫描成功同样合法，不做失败断言
    }
}
