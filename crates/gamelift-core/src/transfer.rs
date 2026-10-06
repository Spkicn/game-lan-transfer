//! 传输引擎：源端临时 SMB 共享、目标端 robocopy 拉取与 Steam acf 认领
//!
//! [`serve`] 在源端建临时只读共享，[`stop_serve`] 删共享
//! [`pull`] 在目标端做空间预检，再经 robocopy 拉取并写认领文件
//!
//! New-SmbShare 与 Remove-SmbShare 需要管理员权限，失败时错误信息会提示以管理员身份运行

use std::path::Path;

use crate::{Error, Result};

/// 临时共享名，清理时按该名字删除
pub const SHARE_NAME: &str = "GameLiftXfer";

/// 源端：给目录建临时只读 SMB 共享，同名共享先删再建
///
/// # Errors
///
/// - [`Error::Shell`]：PowerShell 失败，常见于未提权或路径无效
/// - [`Error::Io`]：目录不存在
pub fn serve(path: &Path) -> Result<()> {
    if !path.is_dir() {
        return Err(Error::Io(format!("共享目录不存在: {}", path.display())));
    }
    let safe = path.display().to_string().replace('\'', "''");
    let script = format!(
        "Remove-SmbShare -Name {SHARE_NAME} -Force -ErrorAction SilentlyContinue; \
         New-SmbShare -Name {SHARE_NAME} -Path '{safe}' -ReadAccess Everyone -ErrorAction Stop"
    );
    let out = crate::shell::hidden_command("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .map_err(|e| Error::Shell(e.to_string()))?;
    if !out.status.success() {
        return Err(Error::Shell(hint_admin(String::from_utf8_lossy(
            &out.stderr,
        ))));
    }
    Ok(())
}

/// 源端：删临时共享，共享不存在时视为成功
///
/// # Errors
///
/// [`Error::Shell`]：PowerShell 失败，常见于未提权
pub fn stop_serve() -> Result<()> {
    let script = format!("Remove-SmbShare -Name {SHARE_NAME} -Force -ErrorAction SilentlyContinue");
    let out = crate::shell::hidden_command("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .map_err(|e| Error::Shell(e.to_string()))?;
    if !out.status.success() {
        return Err(Error::Shell(hint_admin(String::from_utf8_lossy(
            &out.stderr,
        ))));
    }
    Ok(())
}

/// robocopy 退出码语义：0–7 成功，8 及以上失败
#[must_use]
pub fn robocopy_success(exit_code: i32) -> bool {
    (0..8).contains(&exit_code)
}

/// 目标端：从对端共享拉取游戏目录，返回复制字节数
///
/// 先做空间预检，再用 robocopy /E /Z /MT:16 拉取
///
/// # Errors
///
/// - [`Error::InsufficientSpace`]：目标盘空间不足，传输前拦截
/// - [`Error::Shell`]：robocopy 启动失败或退出码 ≥ 8
/// - [`Error::Io`]：目标目录不可写
pub fn pull(
    peer: &str,
    remote_relative: &str,
    dest: &Path,
    drive: &str,
    estimated_bytes: u64,
) -> Result<u64> {
    // 空间预检：预估量 + 512MB 余量
    let margin: u64 = 512 * 1024 * 1024;
    let free = crate::link::free_bytes(drive)?;
    if free < estimated_bytes.saturating_add(margin) {
        return Err(Error::InsufficientSpace(
            estimated_bytes.saturating_add(margin) - free,
        ));
    }
    if let Err(e) = std::fs::create_dir_all(dest) {
        return Err(Error::Io(format!("无法创建目标目录: {e}")));
    }
    let src = format!(r"\\{peer}\{SHARE_NAME}\{remote_relative}");
    let out = crate::shell::hidden_command("robocopy")
        .args([
            &src,
            &dest.display().to_string(),
            "/E",
            "/COPY:DAT",
            "/DCOPY:DAT",
            "/MT:16",
            "/R:2",
            "/W:2",
            "/Z",
            "/NFL",
            "/NDL",
            "/NP",
        ])
        .output()
        .map_err(|e| Error::Shell(format!("robocopy 启动失败: {e}")))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let copied = parse_robocopy_bytes(&stdout);
    if !robocopy_success(out.status.code().unwrap_or(-1)) {
        return Err(Error::Shell(format!(
            "robocopy 失败（退出码 {}）：{}",
            out.status.code().unwrap_or(-1),
            tail_text(&stdout, 5)
        )));
    }
    Ok(copied)
}

/// 从 robocopy 输出统计里解析已复制字节数
/// 中英文系统的行前缀不同，但数值都跟在 "字节"/"Bytes" 标签后的行里
fn parse_robocopy_bytes(stdout: &str) -> u64 {
    // robocopy 摘要行样例：中文 "字节: 64.123 m"，英文 "Bytes : 64.1 m"
    // 单位缩写无法直接 parse，这里逐行匹配标签后取数值段还原
    for line in stdout.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("bytes:") || line.contains("字节:") || line.contains("字节：") {
            // 去掉标签部分，提取"数字[.数字] [k/m/g]"模式
            let rest = line.split(':').nth(1).unwrap_or_default();
            return parse_size_shorthand(rest).unwrap_or(0);
        }
    }
    0
}

/// 解析 robocopy 的容量简写，如 "64.1 m" 转成字节
fn parse_size_shorthand(text: &str) -> Option<u64> {
    let text = text.trim();
    let mut num = String::new();
    let mut unit = ' ';
    for ch in text.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            num.push(ch);
        } else if ch.is_ascii_alphabetic() {
            unit = ch.to_ascii_lowercase();
            break;
        } else if ch.is_whitespace() {
            // "64.1 m" 中的空格：什么都不做，等单位字母
        } else if !num.is_empty() {
            break;
        }
    }
    let value: f64 = num.parse().ok()?;
    let mult: f64 = match unit {
        'k' => 1024.0,
        'm' => 1024.0 * 1024.0,
        'g' => 1024.0 * 1024.0 * 1024.0,
        't' => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };
    // robocopy 的容量缩写本就是估算值，截断精度无影响
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        Some((value * mult) as u64)
    }
}

/// 把 PowerShell 错误输出补上"以管理员身份运行"提示
fn hint_admin(stderr: impl Into<String>) -> String {
    let msg = stderr.into();
    if msg.is_empty() {
        "操作失败，请以管理员身份运行 GameLift".to_owned()
    } else {
        format!("{msg}\n提示: 此操作需要管理员权限（以管理员身份重新运行）")
    }
}

/// 取文本最后 n 行，用于截断错误信息
fn tail_text(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// 生成传完即玩的收尾指引文本
#[must_use]
pub fn claim_instructions(appid: &str, installdir: &str) -> String {
    format!(
        "传输完成，收尾步骤：\n\
         1. 确认 steamapps\\appmanifest_{appid}.acf 与 steamapps\\common\\{installdir} 均已就位\n\
         2. 重启 Steam 客户端\n\
         3. 库中右键游戏 → 属性 → 已安装文件 → 验证文件完整性（buildid 一致时应接近零下载）"
    )
}

/// 源端 steamapps 路径下的 appmanifest 文件名
#[must_use]
pub fn acf_filename(appid: &str) -> String {
    format!("appmanifest_{appid}.acf")
}

/// 目标端认领：把源端 acf 原样复制到目标 steamapps
///
/// # Errors
///
/// [`Error::Io`]：读源或写目标失败
pub fn copy_acf(source_acf: &Path, dest_steamapps: &Path, appid: &str) -> Result<()> {
    let content = std::fs::read_to_string(source_acf)
        .map_err(|e| Error::Io(format!("读取 acf 失败: {e}")))?;
    let dest = dest_steamapps.join(acf_filename(appid));
    // 先写临时名再原子改名，避免 Steam 读到半个 acf
    let tmp = dest.with_extension("acf.tmp");
    std::fs::write(&tmp, content).map_err(|e| Error::Io(format!("写 acf 失败: {e}")))?;
    // Windows 上目标已存在时 rename 会失败，回退为删除后改名
    if std::fs::rename(&tmp, &dest).is_err() {
        let _ = std::fs::remove_file(&dest);
        std::fs::rename(&tmp, &dest).map_err(|e| Error::Io(format!("acf 改名失败: {e}")))?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn robocopy_exit_codes() {
        assert!(robocopy_success(0));
        assert!(robocopy_success(1));
        assert!(robocopy_success(7));
        assert!(!robocopy_success(8));
        assert!(!robocopy_success(16));
        assert!(!robocopy_success(-1));
    }

    #[test]
    fn size_shorthand_parsing() {
        assert_eq!(parse_size_shorthand("0"), Some(0));
        assert_eq!(parse_size_shorthand("1024"), Some(1024));
        assert_eq!(parse_size_shorthand("1 k"), Some(1024));
        assert_eq!(parse_size_shorthand("2 m"), Some(2 * 1024 * 1024));
        assert_eq!(parse_size_shorthand("1.5 g"), Some(1_610_612_736));
        assert_eq!(parse_size_shorthand(""), None);
        assert_eq!(parse_size_shorthand("abc"), None);
    }

    #[test]
    fn acf_filename_format() {
        assert_eq!(acf_filename("123456"), "appmanifest_123456.acf");
    }

    #[test]
    fn copy_acf_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("gamelift-acf-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("steamapps")).expect("mkdir");
        let src_acf = tmp.join("appmanifest_123456.acf");
        std::fs::write(&src_acf, "\"AppState\" { \"appid\" \"123456\" }\n").expect("write");
        copy_acf(&src_acf, &tmp.join("steamapps"), "123456").expect("copy");
        let copied =
            std::fs::read_to_string(tmp.join("steamapps/appmanifest_123456.acf")).expect("read");
        assert!(copied.contains("123456"));
        // 覆盖已有 acf，走 rename 回退路径
        copy_acf(&src_acf, &tmp.join("steamapps"), "123456").expect("overwrite");
        std::fs::remove_dir_all(&tmp).expect("cleanup");
    }
}
