//! 局域网搬运：对端发现、分块传输、逐块校验、断点续传与落盘认领
//!
//! 传输走多个 `TCP` 连接并发拉取分块，每块附带 `blake3` 摘要，接收端逐块校验
//! 续传状态写在暂存目录旁，进程重启后按位图跳过已完成分块
//! 源端只监听调用方给出的直连地址，且只暴露被选中的目录

use std::time::Duration;

use crate::Error;

pub mod client;
pub mod diff;
pub mod discovery;
pub mod frame;
pub mod paths;
pub mod protocol;
pub mod resume;
pub mod server;

/// 分块默认大小，4 MiB
pub const DEFAULT_CHUNK_BYTES: u32 = 4 * 1024 * 1024;

/// 允许的分块下限
pub const MIN_CHUNK_BYTES: u32 = 64 * 1024;

/// 允许的分块上限，受单帧负载上限约束
pub const MAX_CHUNK_BYTES: u32 = 8 * 1024 * 1024;

/// 并发连接默认数量
pub const DEFAULT_STREAMS: usize = 4;

/// 并发连接上限
pub const MAX_STREAMS: usize = 32;

/// 传输会话默认端口
pub const DEFAULT_SESSION_PORT: u16 = 27101;

/// 单帧负载上限，防止对端用超大长度头吃内存
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// 校验摘要长度，固定 32 字节
pub const DIGEST_BYTES: usize = 32;

/// 暂存目录前缀
pub const STAGING_PREFIX: &str = ".gamelift-part-";

/// 非阻塞轮询间隔
pub const IO_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// 把分块大小收敛到允许区间
#[must_use]
pub fn clamp_chunk_bytes(requested: u32) -> u32 {
    requested.clamp(MIN_CHUNK_BYTES, MAX_CHUNK_BYTES)
}

/// 把并发数收敛到允许区间
#[must_use]
pub fn clamp_streams(requested: usize) -> usize {
    requested.clamp(1, MAX_STREAMS)
}

/// 把 `std::io::Error` 转成 crate 错误，直接作为 `map_err` 的处理函数使用
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn io_error(err: std::io::Error) -> Error {
    Error::Io(err.to_string())
}

/// 取互斥锁，锁中毒时继续使用内部值
pub(crate) fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_bytes_are_clamped() {
        assert_eq!(clamp_chunk_bytes(0), MIN_CHUNK_BYTES);
        assert_eq!(clamp_chunk_bytes(u32::MAX), MAX_CHUNK_BYTES);
        assert_eq!(clamp_chunk_bytes(DEFAULT_CHUNK_BYTES), DEFAULT_CHUNK_BYTES);
    }

    #[test]
    fn streams_are_clamped() {
        assert_eq!(clamp_streams(0), 1);
        assert_eq!(clamp_streams(usize::MAX), MAX_STREAMS);
        assert_eq!(clamp_streams(4), 4);
    }
}
