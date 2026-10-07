//! 目录大小统计
//!
//! 目录大小不放进目录列举里，单独按需算：列举要快，算大小要准

use std::path::Path;

/// 递归统计目录下所有文件的总字节数
///
/// 不跟随重解析点，避免符号链接成环；不可读的条目按 0 计，不中断整体统计
#[must_use]
pub fn dir_size(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    let mut total = 0_u64;
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            total = total.saturating_add(dir_size(&entry.path()));
        } else if let Ok(meta) = entry.metadata() {
            total = total.saturating_add(meta.len());
        }
    }
    total
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 建一个层次固定的临时目录
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gamelift-size-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("inner/deeper")).expect("mkdir");
        dir
    }

    #[test]
    fn sums_files_at_every_depth() {
        let dir = scratch("sum");
        std::fs::write(dir.join("top.bin"), vec![0_u8; 100]).expect("write");
        std::fs::write(dir.join("inner/one.bin"), vec![0_u8; 2048]).expect("write");
        std::fs::write(dir.join("inner/deeper/two.bin"), vec![0_u8; 4096]).expect("write");
        assert_eq!(dir_size(&dir), 100 + 2048 + 4096);
        // 单个文件也能算
        assert_eq!(dir_size(&dir.join("top.bin")), 100);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_path_is_zero_not_a_panic() {
        assert_eq!(dir_size(Path::new("Z:\\gamelift-not-there")), 0);
    }

    #[test]
    fn empty_directory_is_zero() {
        let dir = scratch("empty");
        assert_eq!(dir_size(&dir), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
