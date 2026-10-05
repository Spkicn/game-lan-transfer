//! 控制协议：消息定义与 `JSON` 编解码

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// 协议版本，双方不一致时拒绝会话
pub const PROTOCOL_VERSION: u32 = 1;

/// 清单中的一个文件
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// 相对根目录的路径，统一用 `/` 分隔
    pub path: String,
    /// 字节数
    pub size: u64,
}

/// 内容落盘后要在目标机写入的认领文件
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimFile {
    /// 相对认领根目录的路径
    pub relative_path: String,
    /// 文件内容
    pub content: String,
}

/// 拒绝原因，供客户端给出可执行的提示
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectKind {
    /// 协议版本不一致
    Version,
    /// 配对码不匹配
    Pairing,
    /// 找不到请求的内容
    NotFound,
    /// 源端读取失败
    Source,
}

/// 控制消息
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    /// 客户端问候
    Hello {
        /// 协议版本
        version: u32,
        /// 配对码，源端要求配对时必填
        pairing: Option<String>,
        /// 请求的内容名，通常是 `appid` 或目录名
        want: String,
        /// 是否索要清单，并发连接可省略
        need_manifest: bool,
    },
    /// 源端清单
    Manifest {
        /// 展示标题
        title: String,
        /// 平台标识
        platform: String,
        /// 目标根目录名
        root_name: String,
        /// 文件列表
        files: Vec<FileEntry>,
        /// 总字节数
        total_bytes: u64,
        /// 认领文件
        claim_files: Vec<ClaimFile>,
    },
    /// 握手成功但本次连接无需清单
    Ready,
    /// 请求一个分块
    RequestChunk {
        /// 文件在清单中的下标
        file: u32,
        /// 块内起始偏移
        offset: u64,
        /// 块长度
        len: u32,
    },
    /// 客户端告知本连接结束
    Bye,
    /// 源端拒绝本次会话
    Rejected {
        /// 拒绝原因
        kind: RejectKind,
        /// 面向用户的说明
        message: String,
    },
}

/// 把消息编码为 `JSON` 字节
///
/// # Errors
///
/// 序列化失败返回 [`Error::Protocol`]
pub fn encode(message: &Message) -> Result<Vec<u8>> {
    serde_json::to_vec(message).map_err(|err| Error::Protocol(format!("消息序列化失败: {err}")))
}

/// 从 `JSON` 字节解析消息
///
/// # Errors
///
/// 字节不是合法消息时返回 [`Error::Protocol`]
pub fn decode(bytes: &[u8]) -> Result<Message> {
    serde_json::from_slice(bytes).map_err(|err| Error::Protocol(format!("消息解析失败: {err}")))
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn hello_roundtrip() {
        let message = Message::Hello {
            version: PROTOCOL_VERSION,
            pairing: Some("123456".to_owned()),
            want: "431960".to_owned(),
            need_manifest: true,
        };
        let decoded = decode(&encode(&message).expect("encode")).expect("decode");
        assert_eq!(decoded, message);
    }

    #[test]
    fn manifest_roundtrip_keeps_claim_files() {
        let message = Message::Manifest {
            title: "示例游戏".to_owned(),
            platform: "steam".to_owned(),
            root_name: "example_game".to_owned(),
            files: vec![FileEntry {
                path: "bin/game.exe".to_owned(),
                size: 12,
            }],
            total_bytes: 12,
            claim_files: vec![ClaimFile {
                relative_path: "appmanifest_123456.acf".to_owned(),
                content: "\"AppState\" {}".to_owned(),
            }],
        };
        let decoded = decode(&encode(&message).expect("encode")).expect("decode");
        assert_eq!(decoded, message);
    }

    #[test]
    fn rejected_carries_kind() {
        let message = Message::Rejected {
            kind: RejectKind::Pairing,
            message: "配对码不匹配".to_owned(),
        };
        let json = String::from_utf8(encode(&message).expect("encode")).expect("utf8");
        assert!(json.contains("\"pairing\""), "json = {json}");
        let decoded = decode(json.as_bytes()).expect("decode");
        assert_eq!(decoded, message);
    }

    #[test]
    fn garbage_is_protocol_error() {
        let err = decode(b"not json").expect_err("must fail");
        assert!(matches!(err, Error::Protocol(_)), "got {err:?}");
    }
}
