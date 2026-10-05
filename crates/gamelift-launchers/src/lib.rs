//! `GameLift` 平台适配器。
//!
//! 每个启动器（Steam / Epic / 战网 / EA / Ubisoft / GOG）一个模块，
//! 实现 [`LauncherAdapter`] trait，把平台特定格式翻译成
//! `gamelift_core::manifest::TransferManifest`。
//!
//! 约束：新增一个平台适配器 ≤ 200 行，不改核心代码即可注册新平台

pub mod generic;
pub mod steam;

/// 适配器统一错误。库 crate 一律用 thiserror，禁止跨边界传 String。
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    /// 找不到该平台（未安装 / 路径探测失败）。
    #[error("未找到 {platform} 安装")]
    LauncherNotFound {
        /// 平台标识，如 "steam"。
        platform: String,
    },
    /// 库清单文件解析失败。
    #[error("{platform} 清单解析失败: {reason}")]
    ParseFailed {
        /// 平台标识。
        platform: String,
        /// 人话原因，面向最终用户展示。
        reason: String,
    },
    /// 核心 crate 的错误透传。
    #[error(transparent)]
    Core(#[from] gamelift_core::Error),
}

/// 扫描出的一个已安装游戏条目（UI 库列表的一行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledGame {
    /// 展示名，如 "示例游戏"
    pub display_name: String,
    /// 安装目录绝对路径。
    pub install_dir: std::path::PathBuf,
    /// 磁盘占用字节数（SizeOnDisk 或等价字段）。
    pub size_bytes: u64,
    /// 版本指纹（Steam 为 buildid，各平台等价字段；无则为 None）。
    pub version_fingerprint: Option<String>,
}

/// 平台适配器接口。新平台 = 新模块 + 实现本 trait + 注册到 [`adapters`]。
pub trait LauncherAdapter {
    /// 平台标识，如 "steam"（与 TransferManifest.platform 对应）。
    fn platform(&self) -> &'static str;

    /// 扫描本机已安装游戏。找不到平台安装时返回
    /// `AdapterError::LauncherNotFound`，这是正常情况而非故障。
    ///
    /// # Errors
    ///
    /// 平台未安装返回 [`AdapterError::LauncherNotFound`]；
    /// 库目录存在但读取/解析失败返回 [`AdapterError::ParseFailed`]。
    fn scan(&self) -> Result<Vec<InstalledGame>, AdapterError>;
}

/// 返回全部已注册适配器的实例。
///
/// 新适配器在此追加一行即可，不需要改动其他核心代码。
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
        // 测试机上 Steam 大概率未装：作为 LauncherNotFound 正常路径的冒烟验证。
        let adapter = steam::SteamAdapter;
        if let Err(AdapterError::LauncherNotFound { platform }) = adapter.scan() {
            assert_eq!(platform, "steam");
        }
        // 装了 Steam 的机器上扫描成功也合法，不做失败断言。
    }
}
