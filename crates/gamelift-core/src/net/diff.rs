//! 差异传输：内容定义分块、哈希预交换与计划编解码
//!
//! 接收端把自己已有副本的分块摘要发给源端，源端按新内容做一次分块并比对，
//! 只把缺失块列进计划，命中块由接收端从旧副本本地拷贝
//! 这样源端不需要在传输前为整个内容预计算摘要

use std::collections::HashMap;
use std::io::BufReader;
use std::path::Path;

use crate::{Error, Result};

use super::frame::{KIND_HASHES, KIND_PLAN};
use super::DIGEST_BYTES;

/// 分块下限
pub const MIN_CHUNK: u32 = 16 * 1024;

/// 分块平均值，决定索引大小与命中小改动的粒度
pub const AVG_CHUNK: u32 = 64 * 1024;

/// 分块上限
pub const MAX_CHUNK: u32 = 256 * 1024;

/// 一个分块的位置与长度
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkSpan {
    /// 块在文件中的起始偏移
    pub offset: u64,
    /// 块长度
    pub len: u32,
}

/// 带摘要的分块
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkHash {
    /// 位置与长度
    pub span: ChunkSpan,
    /// `blake3` 摘要
    pub hash: [u8; DIGEST_BYTES],
}

/// 源端算出的差异计划项
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanEntry {
    /// 该块在新内容里的位置
    pub span: ChunkSpan,
    /// 命中接收端第几个旧块，未命中为 `None`
    pub matched: Option<u32>,
}

/// 对一个文件做内容定义分块并算摘要
///
/// # Errors
///
/// 读取失败返回 [`Error::Io`]
pub fn index_file(path: &Path) -> Result<Vec<ChunkHash>> {
    let file = std::fs::File::open(path)
        .map_err(|err| Error::Io(format!("打开 {} 失败: {err}", path.display())))?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    let cdc = fastcdc::v2020::StreamCDC::new(
        reader,
        usize::try_from(MIN_CHUNK).unwrap_or(usize::MAX),
        usize::try_from(AVG_CHUNK).unwrap_or(usize::MAX),
        usize::try_from(MAX_CHUNK).unwrap_or(usize::MAX),
    );
    for item in cdc {
        let chunk =
            item.map_err(|err| Error::Io(format!("分块 {} 失败: {err}", path.display())))?;
        let len = u32::try_from(chunk.length)
            .map_err(|_| Error::Protocol("分块长度超出上限".to_owned()))?;
        out.push(ChunkHash {
            span: ChunkSpan {
                offset: chunk.offset,
                len,
            },
            hash: *blake3::hash(&chunk.data).as_bytes(),
        });
    }
    Ok(out)
}

/// 对内存数据做内容定义分块并算摘要
#[must_use]
pub fn index_bytes(data: &[u8]) -> Vec<ChunkHash> {
    let cdc = fastcdc::v2020::FastCDC::new(
        data,
        usize::try_from(MIN_CHUNK).unwrap_or(usize::MAX),
        usize::try_from(AVG_CHUNK).unwrap_or(usize::MAX),
        usize::try_from(MAX_CHUNK).unwrap_or(usize::MAX),
    );
    cdc.filter_map(|chunk| {
        let end = chunk.offset.checked_add(chunk.length)?;
        let bytes = data.get(chunk.offset..end)?;
        Some(ChunkHash {
            span: ChunkSpan {
                offset: u64::try_from(chunk.offset).unwrap_or(u64::MAX),
                len: u32::try_from(chunk.length).ok()?,
            },
            hash: *blake3::hash(bytes).as_bytes(),
        })
    })
    .collect()
}

/// 摘要到位置的索引，同一摘要保留首次出现的位置
#[must_use]
pub fn locate_hashes(chunks: &[ChunkHash]) -> HashMap<[u8; DIGEST_BYTES], ChunkSpan> {
    let mut map = HashMap::with_capacity(chunks.len());
    for chunk in chunks {
        map.entry(chunk.hash).or_insert(chunk.span);
    }
    map
}

/// 按摘要集合筛选计划：命中的块给出对应下标，未命中的标记为缺失
#[must_use]
pub fn build_plan<S: std::hash::BuildHasher>(
    source: &[ChunkHash],
    wanted: &HashMap<[u8; DIGEST_BYTES], u32, S>,
) -> Vec<PlanEntry> {
    source
        .iter()
        .map(|chunk| PlanEntry {
            span: chunk.span,
            matched: wanted.get(&chunk.hash).copied(),
        })
        .collect()
}

/// 编码哈希清单帧负载
#[must_use]
pub fn encode_hashes(file: u32, hashes: &[[u8; DIGEST_BYTES]]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(8 + hashes.len() * DIGEST_BYTES);
    payload.extend_from_slice(&file.to_le_bytes());
    payload.extend_from_slice(&(u32::try_from(hashes.len()).unwrap_or(u32::MAX)).to_le_bytes());
    for hash in hashes {
        payload.extend_from_slice(hash);
    }
    payload
}

/// 解码哈希清单帧负载
///
/// # Errors
///
/// 负载长度与声明不符时返回 [`Error::Protocol`]
pub fn decode_hashes(payload: &[u8]) -> Result<(u32, Vec<[u8; DIGEST_BYTES]>)> {
    if payload.len() < 8 {
        return Err(Error::Protocol("哈希清单帧过短".to_owned()));
    }
    let file = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let count = u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
    let count = usize::try_from(count).unwrap_or(usize::MAX);
    let expected = 8usize.saturating_add(count.saturating_mul(DIGEST_BYTES));
    if payload.len() != expected {
        return Err(Error::Protocol(format!(
            "哈希清单长度 {} 与声明 {} 不符",
            payload.len(),
            expected
        )));
    }
    let mut hashes = Vec::with_capacity(count);
    for index in 0..count {
        let start = 8 + index * DIGEST_BYTES;
        let mut hash = [0u8; DIGEST_BYTES];
        hash.copy_from_slice(&payload[start..start + DIGEST_BYTES]);
        hashes.push(hash);
    }
    Ok((file, hashes))
}

/// 编码差异计划帧负载，命中下标用 `-1` 表示缺失
#[must_use]
pub fn encode_plan(file: u32, plan: &[PlanEntry]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(8 + plan.len() * 16);
    payload.extend_from_slice(&file.to_le_bytes());
    payload.extend_from_slice(&(u32::try_from(plan.len()).unwrap_or(u32::MAX)).to_le_bytes());
    for entry in plan {
        payload.extend_from_slice(&entry.span.offset.to_le_bytes());
        payload.extend_from_slice(&entry.span.len.to_le_bytes());
        let matched = entry
            .matched
            .map_or(-1, |index| i32::try_from(index).unwrap_or(-1));
        payload.extend_from_slice(&matched.to_le_bytes());
    }
    payload
}

/// 解码差异计划帧负载
///
/// # Errors
///
/// 负载长度与声明不符时返回 [`Error::Protocol`]
pub fn decode_plan(payload: &[u8]) -> Result<(u32, Vec<PlanEntry>)> {
    if payload.len() < 8 {
        return Err(Error::Protocol("差异计划帧过短".to_owned()));
    }
    let file = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let count = u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
    let count = usize::try_from(count).unwrap_or(usize::MAX);
    let expected = 8usize.saturating_add(count.saturating_mul(16));
    if payload.len() != expected {
        return Err(Error::Protocol(format!(
            "差异计划长度 {} 与声明 {} 不符",
            payload.len(),
            expected
        )));
    }
    let mut plan = Vec::with_capacity(count);
    for index in 0..count {
        let start = 8 + index * 16;
        let offset = u64::from_le_bytes(
            payload[start..start + 8]
                .try_into()
                .map_err(|_| Error::Protocol("计划项偏移字段不完整".to_owned()))?,
        );
        let len = u32::from_le_bytes(
            payload[start + 8..start + 12]
                .try_into()
                .map_err(|_| Error::Protocol("计划项长度字段不完整".to_owned()))?,
        );
        let matched = i32::from_le_bytes(
            payload[start + 12..start + 16]
                .try_into()
                .map_err(|_| Error::Protocol("计划项命中字段不完整".to_owned()))?,
        );
        plan.push(PlanEntry {
            span: ChunkSpan { offset, len },
            matched: u32::try_from(matched).ok(),
        });
    }
    Ok((file, plan))
}

/// 帧类型常量集中导出，供服务端与客户端判断
#[must_use]
pub fn is_hashes_frame(kind: u8) -> bool {
    kind == KIND_HASHES
}

/// 是否为差异计划帧
#[must_use]
pub fn is_plan_frame(kind: u8) -> bool {
    kind == KIND_PLAN
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::cast_precision_loss)]
mod tests {
    use super::*;

    #[test]
    fn small_data_is_one_chunk() {
        let chunks = index_bytes(b"hello");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].span.offset, 0);
        assert_eq!(chunks[0].span.len, 5);
    }

    /// 生成不可压缩的确定性数据，避免重复模式干扰切点
    fn pseudo_random(len: usize) -> Vec<u8> {
        let mut state = 0x1234_5678_9abc_def0u64;
        (0..len)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                u8::try_from((state >> 33) % 256).unwrap_or(0)
            })
            .collect()
    }

    #[test]
    fn identical_data_indexes_identically() {
        let data = pseudo_random(400_000);
        let first = index_bytes(&data);
        let second = index_bytes(&data);
        assert_eq!(first, second);
        assert!(first.len() > 1, "大文件应切成多块");
    }

    #[test]
    fn insertion_only_shifts_local_boundaries() {
        let data = pseudo_random(4 * 1024 * 1024);
        let mut modified = data.clone();
        modified.splice(2 * 1024 * 1024..2 * 1024 * 1024, [1u8, 2, 3, 4]);
        let base = locate_hashes(&index_bytes(&data));
        let changed = index_bytes(&modified);
        let reused = changed
            .iter()
            .filter(|chunk| base.contains_key(&chunk.hash))
            .count();
        let ratio = reused as f64 / changed.len() as f64;
        assert!(ratio > 0.9, "插入少量字节后应复用绝大多数块，实际 {ratio}");
    }

    #[test]
    fn appended_data_keeps_earlier_chunks() {
        let data = pseudo_random(4 * 1024 * 1024);
        let mut modified = data.clone();
        modified.extend_from_slice(&pseudo_random(64 * 1024));
        let base = locate_hashes(&index_bytes(&data));
        let changed = index_bytes(&modified);
        let reused = changed
            .iter()
            .filter(|chunk| base.contains_key(&chunk.hash))
            .count();
        let ratio = reused as f64 / changed.len() as f64;
        assert!(ratio > 0.9, "追加数据应保留绝大多数原有块，实际 {ratio}");
    }

    #[test]
    fn plan_marks_missing_chunks() {
        let source = index_bytes(
            &(0..300_000u32)
                .map(|v| (v % 251) as u8)
                .collect::<Vec<u8>>(),
        );
        let wanted = locate_hashes(&source[..source.len() / 2]);
        let wanted: HashMap<[u8; DIGEST_BYTES], u32> = wanted
            .into_keys()
            .enumerate()
            .map(|(index, hash)| (hash, u32::try_from(index).unwrap_or(0)))
            .collect();
        let plan = build_plan(&source, &wanted);
        assert_eq!(plan.len(), source.len());
        assert!(plan.iter().any(|entry| entry.matched.is_some()));
    }

    #[test]
    fn hashes_wire_roundtrip() {
        let hashes = vec![[7u8; DIGEST_BYTES], [9u8; DIGEST_BYTES]];
        let (file, decoded) = decode_hashes(&encode_hashes(3, &hashes)).expect("decode");
        assert_eq!(file, 3);
        assert_eq!(decoded, hashes);
    }

    #[test]
    fn plan_wire_roundtrip() {
        let plan = vec![
            PlanEntry {
                span: ChunkSpan { offset: 0, len: 10 },
                matched: Some(0),
            },
            PlanEntry {
                span: ChunkSpan {
                    offset: 10,
                    len: 20,
                },
                matched: None,
            },
        ];
        let (file, decoded) = decode_plan(&encode_plan(1, &plan)).expect("decode");
        assert_eq!(file, 1);
        assert_eq!(decoded, plan);
    }

    #[test]
    fn malformed_payloads_are_rejected() {
        assert!(decode_hashes(&[0, 0, 0]).is_err());
        assert!(decode_plan(&[0, 0, 0]).is_err());
        let mut payload = encode_hashes(0, &[]);
        payload.push(0);
        assert!(decode_hashes(&payload).is_err());
    }
}
