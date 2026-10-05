//! 断点状态：分块完成位图与状态文件读写
//!
//! 状态文件放在暂存目录旁，进程重启后按位图跳过已完成分块

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// 状态结构版本，字段变更时递增
pub const STATE_VERSION: u32 = 2;

/// 一个分块在文件中的位置与长度
///
/// 固定大小切分时为空，差异传输时记录内容定义分块的真实边界
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    /// 起始偏移
    pub offset: u64,
    /// 长度
    pub len: u32,
}

/// 单个文件的续传状态
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileState {
    /// 相对根目录的路径
    pub path: String,
    /// 字节数
    pub size: u64,
    /// 分块完成位图，十六进制字符串，每字节低位在前
    pub bitmap: String,
    /// 分块边界，空表示按固定大小切分
    #[serde(default)]
    pub spans: Vec<Span>,
}

impl FileState {
    /// 分块总数
    #[must_use]
    pub fn chunk_count(&self, chunk_bytes: u64) -> u64 {
        if self.spans.is_empty() {
            chunk_count(self.size, chunk_bytes)
        } else {
            u64::try_from(self.spans.len()).unwrap_or(u64::MAX)
        }
    }

    /// 第 `index` 块的偏移与长度，越界返回 `None`
    #[must_use]
    pub fn span_at(&self, index: u64, chunk_bytes: u64) -> Option<(u64, u64)> {
        if self.spans.is_empty() {
            if index >= chunk_count(self.size, chunk_bytes) {
                return None;
            }
            let offset = index * chunk_bytes;
            let len = chunk_bytes.min(self.size.saturating_sub(offset));
            return Some((offset, len));
        }
        let span = self
            .spans
            .get(usize::try_from(index).unwrap_or(usize::MAX))?;
        Some((span.offset, u64::from(span.len)))
    }
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
            // 旧版本状态直接作废，按首次传输处理，避免用户面出现无法理解的报错
            return Ok(None);
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
            let chunks = file.chunk_count(self.chunk_bytes);
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
            let chunks = file.chunk_count(self.chunk_bytes);
            let Ok(bitmap) = decode_bitmap(&file.bitmap, bitmap_bytes(chunks)) else {
                continue;
            };
            for index in 0..chunks {
                if bit_get(&bitmap, index) {
                    if let Some((_, len)) = file.span_at(index, self.chunk_bytes) {
                        done += len;
                    }
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
                spans: Vec::new(),
            }],
        )
    }

    fn spanned_state() -> TransferState {
        // 三个不定长分块：0..5、5..9、9..10
        let spans = vec![
            Span { offset: 0, len: 5 },
            Span { offset: 5, len: 4 },
            Span { offset: 9, len: 1 },
        ];
        let mut bitmap = vec![0u8; bitmap_bytes(3)];
        bit_set(&mut bitmap, 0);
        bit_set(&mut bitmap, 2);
        TransferState::new(
            "差异示例".to_owned(),
            4,
            vec![FileState {
                path: "data.bin".to_owned(),
                size: 10,
                bitmap: encode_bitmap(&bitmap),
                spans,
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
    fn spanned_state_uses_real_boundaries() {
        let state = spanned_state();
        let file = &state.files[0];
        assert_eq!(file.chunk_count(state.chunk_bytes), 3);
        assert_eq!(file.span_at(0, state.chunk_bytes), Some((0, 5)));
        assert_eq!(file.span_at(2, state.chunk_bytes), Some((9, 1)));
        assert_eq!(file.span_at(3, state.chunk_bytes), None);
        assert_eq!(state.completed_chunks(), 2);
        assert_eq!(state.completed_bytes(), 6);
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
    fn unsupported_version_is_treated_as_absent() {
        let dir = temp_dir("version");
        let path = state_path(&dir, "x");
        std::fs::write(
            &path,
            r#"{"version":99,"chunk_bytes":4,"title":"t","files":[]}"#,
        )
        .expect("write");
        assert!(TransferState::load(&path).expect("load").is_none());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn old_version_state_is_discarded() {
        let dir = temp_dir("old-version");
        let path = state_path(&dir, "x");
        std::fs::write(
            &path,
            r#"{"version":1,"chunk_bytes":4,"title":"t","files":[{"path":"a","size":4,"bitmap":"01"}]}"#,
        )
        .expect("write");
        assert!(TransferState::load(&path).expect("load").is_none());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
