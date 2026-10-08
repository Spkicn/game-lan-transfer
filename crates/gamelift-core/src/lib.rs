//! `GameLift` 传输引擎
//!
//! 职责：局域网对端发现、分块传输与校验、断点续传、进度上报
//! 本 crate 不依赖任何 UI 框架，供 CLI、Tauri 壳与测试共同复用

pub mod link;
pub mod manifest;
pub mod net;
pub mod shell;
pub mod size;
pub mod transfer;
#[cfg(windows)]
pub mod win;

/// crate 级错误类型，库 crate 一律用 thiserror 且禁止跨边界传 String
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 清单解析失败，涵盖 acf / vdf / item 等格式问题
    #[error("清单解析失败: {0}")]
    ManifestParse(String),

    /// 目标磁盘空间不足，携带缺口字节数供调用方给出建议
    #[error("空间不足，还差 {0} 字节")]
    InsufficientSpace(u64),

    /// 找不到可用的物理以太网口
    #[error("找不到物理以太网口（已排除虚拟网卡与无线）")]
    NicNotFound,

    /// PowerShell / netsh / robocopy 等外部命令执行失败
    #[error("命令执行失败: {0}")]
    Shell(String),

    /// IO 错误
    #[error("IO 错误: {0}")]
    Io(String),

    /// 协议交互失败，涵盖版本不匹配与非法消息
    #[error("协议错误: {0}")]
    Protocol(String),

    /// 配对码不匹配，源端拒绝本次会话
    #[error("配对码不匹配，请核对源端给出的 6 位数字")]
    PairingRejected,

    /// 对端声明的路径越出目标根目录
    #[error("路径越界，已拒绝: {0}")]
    PathEscape(String),

    /// 分块校验失败
    #[error("分块校验失败: {0}")]
    Checksum(String),

    /// 调用方取消传输，续传状态会保留
    #[error("传输已取消，续传状态已保留")]
    Cancelled,
}

/// 便捷 Result 别名
pub type Result<T> = std::result::Result<T, Error>;

/// 传输任务的整体统计快照，UI 每秒刷新一次
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferStats {
    /// 已完成并通过校验的字节数
    pub bytes_done: u64,
    /// 总字节数
    pub bytes_total: u64,
    /// 自开始以来的传输速率，单位字节/秒
    pub bytes_per_sec: u64,
}

impl TransferStats {
    /// 完成比例，取值 0.0–1.0，总量为 0 时视为已完成
    /// u64→f64 超过 52 位尾数的精度损失对进度条无影响，故显式放行
    #[allow(clippy::cast_precision_loss)]
    #[must_use]
    pub fn fraction(&self) -> f64 {
        if self.bytes_total == 0 {
            return 1.0;
        }
        self.bytes_done as f64 / self.bytes_total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fraction_of_empty_task_is_one() {
        let stats = TransferStats {
            bytes_done: 0,
            bytes_total: 0,
            bytes_per_sec: 0,
        };
        assert!((stats.fraction() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn fraction_is_zero_at_start() {
        let stats = TransferStats {
            bytes_done: 0,
            bytes_total: 100,
            bytes_per_sec: 0,
        };
        assert!((stats.fraction() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn fraction_at_halfway() {
        let stats = TransferStats {
            bytes_done: 50,
            bytes_total: 100,
            bytes_per_sec: 10,
        };
        assert!((stats.fraction() - 0.5).abs() < f64::EPSILON);
    }
}
