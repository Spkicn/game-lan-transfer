//! 通用文件夹适配器：把任意目录当作搬运对象，非游戏内容也能传

use crate::{AdapterError, InstalledGame, LauncherAdapter};

/// 手动指定文件夹的适配器。不参与 `scan()`（无内容可枚举），
/// 由 UI/CLI 直接调用 [`GenericAdapter::make_entry`] 生成清单条目。
pub struct GenericAdapter;

impl LauncherAdapter for GenericAdapter {
    fn platform(&self) -> &'static str {
        "generic"
    }

    fn scan(&self) -> Result<Vec<InstalledGame>, AdapterError> {
        Ok(Vec::new())
    }
}

impl GenericAdapter {
    /// 把一个用户指定的目录包装成 `InstalledGame`，供 UI 列表展示。
    /// 目录不存在时返回 `LauncherNotFound`（复用"找不到"语义）。
    ///
    /// # Errors
    ///
    /// 路径不是存在的目录时返回 [`AdapterError::LauncherNotFound`]。
    pub fn make_entry(path: &std::path::Path) -> Result<InstalledGame, AdapterError> {
        if !path.is_dir() {
            return Err(AdapterError::LauncherNotFound {
                platform: "generic".into(),
            });
        }
        Ok(InstalledGame {
            display_name: path.file_name().map_or_else(
                || path.to_string_lossy().into_owned(),
                |n| n.to_string_lossy().into_owned(),
            ),
            install_dir: path.to_path_buf(),
            size_bytes: dir_size(path),
            version_fingerprint: None,
        })
    }
}

/// 递归统计目录字节总数。符号链接不跟随（避免循环与重复计数）。
fn dir_size(path: &std::path::Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue; // 无权限/已消失的子目录跳过，不中断统计
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                stack.push(entry.path());
            } else if let Ok(meta) = entry.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn make_entry_rejects_missing_dir() {
        let err = GenericAdapter::make_entry(PathBuf::from("Z:/nope").as_path())
            .expect_err("missing dir must fail");
        assert!(matches!(err, AdapterError::LauncherNotFound { .. }));
    }

    #[test]
    fn dir_size_counts_nested_files_once() {
        let tmp = std::env::temp_dir().join(format!("gamelift-test-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("a/b")).expect("mkdir");
        std::fs::write(tmp.join("a/one.bin"), [0u8; 1024]).expect("write");
        std::fs::write(tmp.join("a/b/two.bin"), [0u8; 512]).expect("write");
        assert_eq!(dir_size(&tmp), 1536);
        std::fs::remove_dir_all(&tmp).expect("cleanup");
    }
}
