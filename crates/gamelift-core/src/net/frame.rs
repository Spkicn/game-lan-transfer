//! 帧编解码：`u32` 小端长度 + 一字节类型 + 负载
//!
//! 读帧前先校验长度上限，避免对端声明超大负载

use std::io::{Read, Write};

use crate::{Error, Result};

use super::io_error;
use super::MAX_FRAME_BYTES;

/// 控制消息帧
pub const KIND_CONTROL: u8 = 1;

/// 分块数据帧
pub const KIND_CHUNK: u8 = 2;

/// 差异传输的哈希清单帧
pub const KIND_HASHES: u8 = 3;

/// 差异计划帧
pub const KIND_PLAN: u8 = 4;

/// 帧头长度，4 字节长度加 1 字节类型
pub const HEADER_BYTES: usize = 5;

/// 一帧的类型与负载
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// 帧类型
    pub kind: u8,
    /// 帧负载
    pub payload: Vec<u8>,
}

impl Frame {
    /// 构造控制帧
    #[must_use]
    pub fn control(payload: Vec<u8>) -> Self {
        Self {
            kind: KIND_CONTROL,
            payload,
        }
    }

    /// 构造分块帧
    #[must_use]
    pub fn chunk(payload: Vec<u8>) -> Self {
        Self {
            kind: KIND_CHUNK,
            payload,
        }
    }

    /// 是否为控制帧
    #[must_use]
    pub fn is_control(&self) -> bool {
        self.kind == KIND_CONTROL
    }

    /// 是否为分块帧
    #[must_use]
    pub fn is_chunk(&self) -> bool {
        self.kind == KIND_CHUNK
    }
}

/// 写入一帧并冲刷缓冲区
///
/// # Errors
///
/// 负载超过上限返回 [`Error::Protocol`]，写入失败返回 [`Error::Io`]
pub fn write_frame(writer: &mut impl Write, frame: &Frame) -> Result<()> {
    if frame.payload.len() > MAX_FRAME_BYTES {
        return Err(Error::Protocol(format!(
            "帧负载 {} 字节超过上限 {MAX_FRAME_BYTES}",
            frame.payload.len()
        )));
    }
    let len = u32::try_from(frame.payload.len())
        .map_err(|_| Error::Protocol("帧负载超过 4 GiB".to_owned()))?;
    writer.write_all(&len.to_le_bytes()).map_err(io_error)?;
    writer.write_all(&[frame.kind]).map_err(io_error)?;
    writer.write_all(&frame.payload).map_err(io_error)?;
    writer.flush().map_err(io_error)
}

/// 读取一帧，对端正常关闭时返回 `None`
///
/// # Errors
///
/// 长度头非法、负载超限、帧头未读完即断连或读取失败时返回错误
pub fn read_frame(reader: &mut impl Read) -> Result<Option<Frame>> {
    let mut header = [0u8; HEADER_BYTES];
    if !read_header(reader, &mut header)? {
        return Ok(None);
    }
    let len = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let len = usize::try_from(len).map_err(|_| Error::Protocol("非法帧长度".to_owned()))?;
    if len > MAX_FRAME_BYTES {
        return Err(Error::Protocol(format!(
            "帧负载 {len} 字节超过上限 {MAX_FRAME_BYTES}"
        )));
    }
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).map_err(io_error)?;
    Ok(Some(Frame {
        kind: header[4],
        payload,
    }))
}

/// 读满帧头，首个字节即遇到 EOF 时返回 false
fn read_header(reader: &mut impl Read, buf: &mut [u8; HEADER_BYTES]) -> Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        let read = reader.read(&mut buf[filled..]).map_err(io_error)?;
        if read == 0 {
            if filled == 0 {
                return Ok(false);
            }
            return Err(Error::Protocol("帧头未读完连接即关闭".to_owned()));
        }
        filled += read;
    }
    Ok(true)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn control_frame_roundtrip() {
        let frame = Frame::control(b"hello".to_vec());
        let mut buf = Vec::new();
        write_frame(&mut buf, &frame).expect("write");
        let mut cursor = std::io::Cursor::new(buf);
        let decoded = read_frame(&mut cursor).expect("read").expect("frame");
        assert_eq!(decoded, frame);
        assert!(decoded.is_control());
    }

    #[test]
    fn empty_payload_roundtrip() {
        let frame = Frame::chunk(Vec::new());
        let mut buf = Vec::new();
        write_frame(&mut buf, &frame).expect("write");
        let mut cursor = std::io::Cursor::new(buf);
        let decoded = read_frame(&mut cursor).expect("read").expect("frame");
        assert_eq!(decoded.kind, KIND_CHUNK);
        assert!(decoded.payload.is_empty());
    }

    #[test]
    fn clean_eof_yields_none() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        assert!(read_frame(&mut cursor).expect("read").is_none());
    }

    #[test]
    fn oversized_length_header_is_rejected() {
        let mut buf = Vec::new();
        let len = u32::try_from(MAX_FRAME_BYTES + 1).expect("fits");
        buf.extend_from_slice(&len.to_le_bytes());
        buf.push(KIND_CONTROL);
        let mut cursor = std::io::Cursor::new(buf);
        let err = read_frame(&mut cursor).expect_err("must reject");
        assert!(matches!(err, Error::Protocol(_)), "got {err:?}");
    }

    #[test]
    fn truncated_header_is_rejected() {
        let mut cursor = std::io::Cursor::new(vec![1u8, 0, 0]);
        let err = read_frame(&mut cursor).expect_err("must reject");
        assert!(matches!(err, Error::Protocol(_)), "got {err:?}");
    }
}
