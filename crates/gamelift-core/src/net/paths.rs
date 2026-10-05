//! 相对路径净化：拒绝任何越出目标根目录的声明
//!
//! 对端声明的路径一律当作不可信输入，先净化再拼接，拼接后再确认仍在根目录内

use std::path::{Component, Path, PathBuf};

use crate::{Error, Result};

/// Windows 保留设备名，不能作为文件名
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// 校验并规范化对端声明的相对路径
///
/// # Errors
///
/// 空路径、绝对路径、含 `..`、含冒号、含 NUL 或使用保留名时返回 [`Error::PathEscape`]
pub fn sanitize_relative(raw: &str) -> Result<PathBuf> {
    if raw.is_empty() || raw.contains('\0') {
        return Err(Error::PathEscape(raw.to_owned()));
    }
    let normalized = raw.replace('\\', "/");
    if normalized.starts_with('/') {
        return Err(Error::PathEscape(raw.to_owned()));
    }
    let mut out = PathBuf::new();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') || is_reserved(part) {
            return Err(Error::PathEscape(raw.to_owned()));
        }
        out.push(part);
    }
    if out.as_os_str().is_empty() {
        return Err(Error::PathEscape(raw.to_owned()));
    }
    Ok(out)
}

/// 是否为保留名，或带尾随空格与点号的名字
fn is_reserved(part: &str) -> bool {
    if part.ends_with(' ') || part.ends_with('.') {
        return true;
    }
    let stem = part.split('.').next().unwrap_or(part);
    RESERVED.iter().any(|name| stem.eq_ignore_ascii_case(name))
}

/// 把相对路径拼到根目录下，并确认结果仍在根目录内
///
/// # Errors
///
/// 拼接结果越出根目录时返回 [`Error::PathEscape`]
pub fn join_within(root: &Path, relative: &Path) -> Result<PathBuf> {
    let root_parts: Vec<Component<'_>> = root.components().collect();
    let joined = root.join(relative);
    let mut parts: Vec<Component<'_>> = Vec::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.len() <= root_parts.len() {
                    return Err(Error::PathEscape(relative.display().to_string()));
                }
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    let inside = parts.len() > root_parts.len()
        && parts.get(..root_parts.len()) == Some(root_parts.as_slice());
    if !inside {
        return Err(Error::PathEscape(relative.display().to_string()));
    }
    Ok(parts.iter().collect())
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn plain_relative_path_is_kept() {
        let path = sanitize_relative("bin/win64/game.exe").expect("accept");
        assert_eq!(path, PathBuf::from("bin/win64/game.exe"));
    }

    #[test]
    fn backslashes_are_normalized() {
        let path = sanitize_relative(r"data\level1\map.bin").expect("accept");
        assert_eq!(path, PathBuf::from("data/level1/map.bin"));
    }

    #[test]
    fn chinese_names_are_kept() {
        let path = sanitize_relative("素材/地图 01.dat").expect("accept");
        assert_eq!(path, PathBuf::from("素材/地图 01.dat"));
    }

    #[test]
    fn parent_traversal_is_rejected() {
        for raw in [
            "../evil.exe",
            "bin/../../evil.exe",
            r"..\evil.exe",
            "..",
            "bin/..",
        ] {
            let err = sanitize_relative(raw).expect_err("must reject");
            assert!(matches!(err, Error::PathEscape(_)), "{raw} => {err:?}");
        }
    }

    #[test]
    fn absolute_and_drive_paths_are_rejected() {
        for raw in [
            "/etc/passwd",
            r"\windows\system32",
            "C:/windows",
            "C:evil.exe",
        ] {
            let err = sanitize_relative(raw).expect_err("must reject");
            assert!(matches!(err, Error::PathEscape(_)), "{raw} => {err:?}");
        }
    }

    #[test]
    fn reserved_names_and_odd_suffixes_are_rejected() {
        for raw in [
            "NUL",
            "con.txt",
            "aux",
            "bin/lpt1.bin",
            "trailing ",
            "trailing.",
        ] {
            let err = sanitize_relative(raw).expect_err("must reject");
            assert!(matches!(err, Error::PathEscape(_)), "{raw} => {err:?}");
        }
    }

    #[test]
    fn empty_and_nul_are_rejected() {
        assert!(sanitize_relative("").is_err());
        assert!(sanitize_relative("a\0b").is_err());
        assert!(sanitize_relative("./").is_err());
    }

    #[test]
    fn join_within_keeps_paths_inside_root() {
        let root = Path::new("D:/games/example");
        let joined = join_within(root, Path::new("bin/game.exe")).expect("accept");
        assert_eq!(joined, PathBuf::from("D:/games/example/bin/game.exe"));
    }

    #[test]
    fn join_within_rejects_escape_and_root_itself() {
        let root = Path::new("D:/games/example");
        assert!(join_within(root, Path::new("../other")).is_err());
        assert!(join_within(root, Path::new("")).is_err());
    }
}
