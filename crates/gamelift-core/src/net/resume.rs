//! 断点状态：分块完成位图与状态文件读写
//!
//! 状态文件放在暂存目录旁，进程重启后按位图跳过已完成分块

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// 状态结构版本，字段变更时递增
pub const STATE_VERSION: u32 = 1;

/// 单个文件的续传状态
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileState {
    /// 相对根目录的路径
    pub path: String,
    /// 字节数
    pub size: u64,
    /// 分块完成位图，十六进制字符串，每字节低位在前
    pub bitmap: String,
}

/// 一次传输的续传状态
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferState {
    /// 结构版本
    pub version: u32,
    /// 分块大小
    pub chunk_bytes: u64,
    /// 展示标题
    pub title: String,
    /// 文件状态
    pub files: Vec<FileState>,
}

/// 状态文件路径，按内容名区分同目录下的多次传输
#[must_use]
pub fn state_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!(".gamelift-state-{name}.json"))
}

/// 文件需要的分块数量
#[must_use]
pub fn chunk_count(size: u64, chunk_bytes: u64) -> u64 {
    if chunk_bytes == 0 {
        return 0;
    }
    size.div_ceil(chunk_bytes)
}

/// 位图需要的字节数
#[must_use]
pub fn bitmap_bytes(chunks: u64) -> usize {
    usize::try_from(chunks.div_ceil(8)).unwrap_or(usize::MAX)
}

/// 读取某一位，越界视为未完成
#[must_use]
pub fn bit_get(bitmap: &[u8], index: u64) -> bool {
    let byte = usize::try_from(index / 8).unwrap_or(usize::MAX);
    let bit = u32::try_from(index % 8).unwrap_or(0);
    bitmap.get(byte).is_some_and(|slot| slot & (1 << bit) != 0)
}

/// 置位某一位，越界忽略
pub fn bit_set(bitmap: &mut [u8], index: u64) {
    let byte = usize::try_from(index / 8).unwrap_or(usize::MAX);
    let bit = u32::try_from(index % 8).unwrap_or(0);
    if let Some(slot) = bitmap.get_mut(byte) {
        *slot |= 1 << bit;
    }
}

/// 清位，越界忽略
pub fn bit_clear(bitmap: &mut [u8], index: u64) {
    let byte = usize::try_from(index / 8).unwrap_or(usize::MAX);
    let bit = u32::try_from(index % 8).unwrap_or(0);
    if let Some(slot) = bitmap.get_mut(byte) {
        *slot &= !(1 << bit);
    }
}

/// 位图转十六进制字符串
#[must_use]
pub fn encode_bitmap(bits: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bits.len() * 2);
    for byte in bits {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

/// 十六进制字符串转位图
///
/// # Errors
///
/// 长度与期望不符或含非法字符时返回 [`Error::Protocol`]
pub fn decode_bitmap(hex: &str, bytes: usize) -> Result<Vec<u8>> {
    if hex.len() != bytes * 2 {
        return Err(Error::Protocol(format!(
            "续传位图长度 {} 与期望 {bytes} 字节不符",
            hex.len()
        )));
    }
    let raw = hex.as_bytes();
    let mut out = Vec::with_capacity(bytes);
    for index in 0..bytes {
        let high = hex_value(raw[index * 2])?;
        let low = hex_value(raw[index * 2 + 1])?;
        out.push((high << 4) | low);
    }
    Ok(out)
}

/// 单个十六进制字符转数值
fn hex_value(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        other => Err(Error::Protocol(format!(
            "续传位图含非法字符 {}",
            char::from(other)
        ))),
    }
}

impl TransferState {
    /// 按清单构造空状态
    #[must_use]
    pub fn new(title: String, chunk_bytes: u64, files: Vec<FileState>) -> Self {
        Self {
            version: STATE_VERSION,
            chunk_bytes,
            title,
            files,
        }
    }

    /// 读取状态，文件不存在返回 `None`
    ///
    /// # Errors
    ///
    /// 文件存在但不是本版本合法状态时返回 [`Error::Protocol`]
    pub fn load(path: &Path) -> Result<Option<Self>> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(Error::Io(format!("读取续传状态失败: {err}"))),
        };
        let state: Self = serde_json::from_str(&text)
            .map_err(|err| Error::Protocol(format!("续传状态解析失败: {err}")))?;
        if state.version != STATE_VERSION {
            return Err(Error::Protocol(format!(
                "续传状态版本 {} 不受支持",
                state.version
            )));
        }
        Ok(Some(state))
    }

    /// 写入状态文件，先写临时名再改名，避免进程中途被杀留下半个文件
    ///
    /// # Errors
    ///
    /// 目录不可写或改名失败时返回 [`Error::Io`]
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| Error::Io(format!("创建暂存目录失败: {err}")))?;
        }
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string(self)
            .map_err(|err| Error::Protocol(format!("续传状态序列化失败: {err}")))?;
        std::fs::write(&tmp, text).map_err(|err| Error::Io(format!("写入续传状态失败: {err}")))?;
        if std::fs::rename(&tmp, path).is_err() {
            let _ = std::fs::remove_file(path);
            std::fs::rename(&tmp, path)
                .map_err(|err| Error::Io(format!("续传状态改名失败: {err}")))?;
        }
        Ok(())
    }

    /// 删除状态文件，文件本来不存在视为成功
    ///
    /// # Errors
    ///
    /// 删除失败且非"文件不存在"时返回 [`Error::Io`]
    pub fn remove(path: &Path) -> Result<()> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(Error::Io(format!("删除续传状态失败: {err}"))),
        }
    }

    /// 已完成分块数量
    #[must_use]
    pub fn completed_chunks(&self) -> u64 {
        let mut done = 0;
        for file in &self.files {
            let chunks = chunk_count(file.size, self.chunk_bytes);
            let Ok(bitmap) = decode_bitmap(&file.bitmap, bitmap_bytes(chunks)) else {
                continue;
            };
            done += (0..chunks).filter(|index| bit_get(&bitmap, *index)).count() as u64;
        }
        done
    }

    /// 已完成字节数
    #[must_use]
    pub fn completed_bytes(&self) -> u64 {
        let mut done = 0;
        for file in &self.files {
            let chunks = chunk_count(file.size, self.chunk_bytes);
            let Ok(bitmap) = decode_bitmap(&file.bitmap, bitmap_bytes(chunks)) else {
                continue;
            };
            for index in 0..chunks {
                if bit_get(&bitmap, index) {
                    let offset = index * self.chunk_bytes;
                    done += self.chunk_bytes.min(file.size.saturating_sub(offset));
                }
            }
        }
        done
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gamelift-state-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn chunk_count_rounds_up() {
        assert_eq!(chunk_count(0, 100), 0);
        assert_eq!(chunk_count(1, 100), 1);
        assert_eq!(chunk_count(100, 100), 1);
        assert_eq!(chunk_count(101, 100), 2);
    }

    #[test]
    fn bitmap_set_get_clear() {
        let mut bitmap = vec![0u8; bitmap_bytes(20)];
        assert!(!bit_get(&bitmap, 0));
        bit_set(&mut bitmap, 0);
        bit_set(&mut bitmap, 9);
        bit_clear(&mut bitmap, 0);
        assert!(!bit_get(&bitmap, 0));
        assert!(bit_get(&bitmap, 9));
        assert!(!bit_get(&bitmap, 1));
        assert!(!bit_get(&bitmap, 999));
    }

    #[test]
    fn bitmap_hex_roundtrip() {
        let mut bitmap = vec![0u8; 4];
        bit_set(&mut bitmap, 0);
        bit_set(&mut bitmap, 17);
        let hex = encode_bitmap(&bitmap);
        assert_eq!(hex.len(), 8);
        assert_eq!(decode_bitmap(&hex, 4).expect("decode"), bitmap);
    }

    #[test]
    fn bitmap_decode_rejects_bad_input() {
        assert!(decode_bitmap("abc", 2).is_err());
        assert!(decode_bitmap("zz", 1).is_err());
    }

    fn sample_state() -> TransferState {
        let chunks = chunk_count(10, 4);
        let mut bitmap = vec![0u8; bitmap_bytes(chunks)];
        bit_set(&mut bitmap, 0);
        TransferState::new(
            "示例游戏".to_owned(),
            4,
            vec![FileState {
                path: "data.bin".to_owned(),
                size: 10,
                bitmap: encode_bitmap(&bitmap),
            }],
        )
    }

    #[test]
    fn state_counts_completed_work() {
        let state = sample_state();
        assert_eq!(state.completed_chunks(), 1);
        assert_eq!(state.completed_bytes(), 4);
    }

    #[test]
    fn state_save_load_roundtrip() {
        let dir = temp_dir("roundtrip");
        let path = state_path(&dir, "示例游戏");
        let state = sample_state();
        state.save(&path).expect("save");
        let loaded = TransferState::load(&path).expect("load").expect("present");
        assert_eq!(loaded, state);
        TransferState::remove(&path).expect("remove");
        assert!(TransferState::load(&path).expect("load").is_none());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn load_missing_file_is_none() {
        let dir = temp_dir("missing");
        assert!(TransferState::load(&state_path(&dir, "nope"))
            .expect("load")
            .is_none());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let dir = temp_dir("version");
        let path = state_path(&dir, "x");
        std::fs::write(
            &path,
            r#"{"version":99,"chunk_bytes":4,"title":"t","files":[]}"#,
        )
        .expect("write");
        assert!(TransferState::load(&path).is_err());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
