//! 局域网对端发现：`UDP` 广播公告与收集
//!
//! 公告只承载"我是谁、在哪个端口等着"，真正的连接仍走 `TCP`
//! 发送端绑定调用方给出的直连地址，不向默认路由所在接口广播

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// 发现通道默认端口
pub const DISCOVERY_PORT: u16 = 27100;

/// 公告负载上限
pub const MAX_ANNOUNCE_BYTES: usize = 1024;

/// 局域网上一个可连接的对端
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    /// 本次运行的实例号，用于忽略自己发出的公告
    pub instance: u64,
    /// 主机名
    pub name: String,
    /// 对端地址
    pub addr: IpAddr,
    /// 传输会话端口
    pub session_port: u16,
    /// 平台标识
    pub platform: String,
    /// 接收盘可用空间
    pub free_bytes: u64,
    /// 是否要求配对码
    pub pairing_required: bool,
}

/// 生成一次性实例号
#[must_use]
pub fn new_instance() -> u64 {
    rand::random()
}

/// 生成 6 位配对码
#[must_use]
pub fn new_pairing_code() -> String {
    let value: u32 = rand::random::<u32>() % 1_000_000;
    format!("{value:06}")
}

/// 本机名，取不到环境变量时退回固定名
#[must_use]
pub fn local_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "GameLift".to_owned())
}

/// 当前平台标识
#[must_use]
pub fn local_platform() -> String {
    std::env::consts::OS.to_owned()
}

/// 编码公告负载
///
/// # Errors
///
/// 编码失败或超过长度上限时返回 [`Error::Protocol`]
pub fn encode_announce(peer: &Peer) -> Result<Vec<u8>> {
    let bytes =
        serde_json::to_vec(peer).map_err(|err| Error::Protocol(format!("公告编码失败: {err}")))?;
    if bytes.len() > MAX_ANNOUNCE_BYTES {
        return Err(Error::Protocol("公告负载超过上限".to_owned()));
    }
    Ok(bytes)
}

/// 解析公告，非法负载返回 `None`
#[must_use]
pub fn decode_announce(bytes: &[u8]) -> Option<Peer> {
    if bytes.len() > MAX_ANNOUNCE_BYTES {
        return None;
    }
    serde_json::from_slice(bytes).ok()
}

/// 绑定发现端口并开启广播发送
///
/// # Errors
///
/// 绑定或套接字配置失败返回 [`Error::Io`]
pub fn bind(local_ip: IpAddr, port: u16) -> Result<UdpSocket> {
    let socket = UdpSocket::bind(SocketAddr::new(local_ip, port)).map_err(|err| {
        if err.kind() == std::io::ErrorKind::AddrNotAvailable {
            Error::Io(format!(
                "本机地址 {local_ip} 现在不在任何网卡上，可能刚改过网络设置：点「重新检测」刷新后再试"
            ))
        } else {
            Error::Io(format!("绑定发现端口失败: {err}"))
        }
    })?;
    socket
        .set_broadcast(true)
        .map_err(|err| Error::Io(format!("开启广播失败: {err}")))?;
    Ok(socket)
}

/// 向目标地址发送一次公告
///
/// # Errors
///
/// 编码失败或发送失败时返回错误
pub fn announce_to(socket: &UdpSocket, target: SocketAddr, peer: &Peer) -> Result<()> {
    let payload = encode_announce(peer)?;
    socket
        .send_to(&payload, target)
        .map_err(|err| Error::Io(format!("发送公告失败: {err}")))?;
    Ok(())
}

/// 由主机地址推出 `24` 位掩码下的定向广播地址
#[must_use]
pub fn directed_broadcast(ip: Ipv4Addr) -> Ipv4Addr {
    let octets = ip.octets();
    Ipv4Addr::new(octets[0], octets[1], octets[2], 255)
}

/// 在超时时间内收集对端，按地址与端口去重并忽略指定实例
///
/// # Errors
///
/// 套接字配置失败返回 [`Error::Io`]；坏数据报直接丢弃
pub fn collect(
    socket: &UdpSocket,
    timeout: Duration,
    self_instance: Option<u64>,
) -> Result<Vec<Peer>> {
    let deadline = Instant::now() + timeout;
    let mut peers: Vec<Peer> = Vec::new();
    let mut buf = vec![0u8; MAX_ANNOUNCE_BYTES];
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        socket
            .set_read_timeout(Some(remaining.max(Duration::from_millis(1))))
            .map_err(|err| Error::Io(format!("设置读取超时失败: {err}")))?;
        match socket.recv_from(&mut buf) {
            Ok((len, _from)) => {
                let Some(peer) = decode_announce(buf.get(..len).unwrap_or_default()) else {
                    continue;
                };
                if Some(peer.instance) == self_instance {
                    continue;
                }
                let duplicate = peers
                    .iter()
                    .any(|seen| seen.addr == peer.addr && seen.session_port == peer.session_port);
                if !duplicate {
                    peers.push(peer);
                }
            }
            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock
                    || err.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            // 截断或乱序的数据报直接丢弃，继续等
            Err(_) => {}
        }
    }
    Ok(peers)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn sample_peer() -> Peer {
        Peer {
            instance: 42,
            name: "workstation".to_owned(),
            addr: IpAddr::V4(Ipv4Addr::new(192, 168, 88, 1)),
            session_port: 27101,
            platform: "windows".to_owned(),
            free_bytes: 1 << 30,
            pairing_required: true,
        }
    }

    #[test]
    fn announce_roundtrip() {
        let peer = sample_peer();
        let decoded = decode_announce(&encode_announce(&peer).expect("encode")).expect("decode");
        assert_eq!(decoded, peer);
    }

    #[test]
    fn garbage_announce_is_none() {
        assert!(decode_announce(b"{}").is_none());
        assert!(decode_announce(b"not json").is_none());
    }

    #[test]
    fn oversized_announce_is_rejected() {
        let payload = vec![b'x'; MAX_ANNOUNCE_BYTES + 1];
        assert!(decode_announce(&payload).is_none());
    }

    #[test]
    fn directed_broadcast_keeps_prefix() {
        assert_eq!(
            directed_broadcast(Ipv4Addr::new(192, 168, 88, 2)),
            Ipv4Addr::new(192, 168, 88, 255)
        );
    }

    #[test]
    fn pairing_code_has_six_digits() {
        for _ in 0..32 {
            let code = new_pairing_code();
            assert_eq!(code.len(), 6, "code = {code}");
            assert!(code.chars().all(|ch| ch.is_ascii_digit()), "code = {code}");
        }
    }

    #[test]
    fn instance_ids_differ() {
        assert_ne!(new_instance(), new_instance());
    }

    #[test]
    fn announce_is_collected_on_loopback() {
        let sender = bind(IpAddr::V4(Ipv4Addr::LOCALHOST), 0).expect("bind sender");
        let listener = bind(IpAddr::V4(Ipv4Addr::LOCALHOST), 0).expect("bind listener");
        let target = listener.local_addr().expect("addr");
        let peer = sample_peer();
        announce_to(&sender, target, &peer).expect("announce");
        let found = collect(&listener, Duration::from_millis(500), None).expect("collect");
        assert_eq!(found, vec![peer]);
    }

    #[test]
    fn own_announce_is_ignored() {
        let sender = bind(IpAddr::V4(Ipv4Addr::LOCALHOST), 0).expect("bind sender");
        let listener = bind(IpAddr::V4(Ipv4Addr::LOCALHOST), 0).expect("bind listener");
        let peer = sample_peer();
        announce_to(&sender, listener.local_addr().expect("addr"), &peer).expect("announce");
        let found =
            collect(&listener, Duration::from_millis(200), Some(peer.instance)).expect("collect");
        assert_eq!(found, Vec::<Peer>::new(), "found = {found:?}");
    }

    #[test]
    fn collect_returns_empty_when_nobody_announces() {
        let listener = bind(IpAddr::V4(Ipv4Addr::LOCALHOST), 0).expect("bind listener");
        let found = collect(&listener, Duration::from_millis(120), None).expect("collect");
        assert_eq!(found, Vec::<Peer>::new());
    }
}
