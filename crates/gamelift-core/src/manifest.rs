//! 传输清单：待搬运内容的统一描述。
//!
//! 各平台适配器（gamelift-launchers）负责把 Steam acf / Epic .item 等
//! 平台特定格式翻译成这里的统一清单，传输引擎只认这里的类型。

use std::path::PathBuf;

/// 一次搬运任务的清单：要传哪些内容、预计多大、指纹是什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferManifest {
    /// 人类可读的任务名，如 "示例游戏 (Steam)"
    pub title: String,
    /// 平台来源标识，如 "steam" / "epic" / "generic"（手动添加的任意文件夹）。
    pub platform: String,
    /// 版本指纹。对 Steam 是 `buildid` + 各 depot manifest id 的组合；
    /// 指纹完全一致时传输引擎可直接走"零差异快速路径"。
    pub version_fingerprint: Option<String>,
    /// 源端条目列表（文件或递归目录）。
    pub entries: Vec<ManifestEntry>,
    /// 传输完成后，目标机需要落盘的清单文件（相对各自平台根）。
    /// 传输引擎在全部内容校验通过后才写这些文件（原子提交的最后一步）。
    pub claim_files: Vec<ClaimFile>,
}

/// 清单中的单个源端条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    /// 源端绝对路径（文件或目录；目录递归展开）。
    pub source: PathBuf,
    /// 在目标端的相对路径。
    pub relative_path: PathBuf,
    /// 预估字节数（目录为其递归总量），用于空间预检与进度分母。
    pub estimated_bytes: u64,
}

/// 传输完成后在目标机写入的"认领"文件（如 `appmanifest_<appid>.acf`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimFile {
    /// 目标平台根下的相对路径，如 `steamapps/appmanifest_123456.acf`
    pub relative_path: PathBuf,
    /// 文件内容（适配器生成，引擎原样落盘）。
    pub content: String,
}

impl TransferManifest {
    /// 全部条目的预估总字节数。UI 在用户点"开始"前必须展示这个值。
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.entries.iter().map(|e| e.estimated_bytes).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(bytes: u64) -> ManifestEntry {
        ManifestEntry {
            source: PathBuf::from("C:/games/foo"),
            relative_path: PathBuf::from("foo"),
            estimated_bytes: bytes,
        }
    }

    #[test]
    fn total_bytes_sums_entries() {
        let m = TransferManifest {
            title: "t".into(),
            platform: "steam".into(),
            version_fingerprint: None,
            entries: vec![entry(100), entry(23)],
            claim_files: vec![],
        };
        assert_eq!(m.total_bytes(), 123);
    }

    #[test]
    fn empty_manifest_has_zero_total() {
        let m = TransferManifest {
            title: "t".into(),
            platform: "generic".into(),
            version_fingerprint: None,
            entries: vec![],
            claim_files: vec![],
        };
        assert_eq!(m.total_bytes(), 0);
    }
}
