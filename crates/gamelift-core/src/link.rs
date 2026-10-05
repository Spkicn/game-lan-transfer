//! 直连自举：识别物理以太网口、配置静态地址、还原
//!
//! 网络配置走 PowerShell，提权由调用方负责；已有可用 IPv4 的网口跳过

use crate::{Error, Result};

/// 直连网段，默认 192.168.88.0/24
pub const LINK_SUBNET_PREFIX: &str = "192.168.88";

/// 一个候选网口的描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nic {
    /// Windows 网络连接名，如 "以太网"。
    pub name: String,
    /// 硬件描述，如 "Intel(R) Ethernet Connection I219-V"。
    pub description: String,
    /// 是否为物理以太网口（已排除虚拟网卡与 WLAN）。
    pub is_physical: bool,
    /// 现有 IPv4 地址（LinkLocal 169.254 除外）。None = 未配置。
    pub current_ipv4: Option<String>,
}

/// 罗列所有网口。单次 PowerShell 调用取齐名称、描述、IP（无需管理员）。
/// 输出编码显式设为 UTF-8，避免中文 Windows 控制台 GBK 乱码。
#[must_use]
pub fn list_nics() -> Vec<Nic> {
    let ps = "[Console]::OutputEncoding=[Text.Encoding]::UTF8; \
              Get-NetAdapter | ForEach-Object { \
                $n=$_.Name; \
                $ip=(Get-NetIPAddress -InterfaceAlias $n -AddressFamily IPv4 -ErrorAction SilentlyContinue \
                     | Where-Object { $_.IPAddress -notlike '169.254*' -and $_.IPAddress -notlike '127.*' } \
                     | Select-Object -First 1).IPAddress; \
                '{0}|{1}|{2}' -f $n, $_.InterfaceDescription, $ip \
              }";
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", ps])
        .output();
    let Ok(out) = output else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut nics = Vec::new();
    for line in text.lines() {
        let Some((name, desc, ip)) = split_nic_line(line) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        nics.push(Nic {
            current_ipv4: ip,
            is_physical: is_physical_ethernet(&name, &desc),
            description: desc,
            name,
        });
    }
    nics
}

/// 拆 `名称|描述|IP` 行（从右取：最后一段是 IP，IP 可为空）。
/// 描述不含 `|`（`InterfaceDescription` 实际不含）；万一含，靠 rsplitn 保证 IP 仍正确。
fn split_nic_line(line: &str) -> Option<(String, String, Option<String>)> {
    let (head, ip_raw) = line.rsplit_once('|')?;
    let (name_raw, desc_raw) = head.split_once('|')?;
    let ip = if ip_raw.trim().is_empty() {
        None
    } else {
        Some(ip_raw.trim().to_owned())
    };
    Some((name_raw.trim().to_owned(), desc_raw.trim().to_owned(), ip))
}

/// 物理以太网判定：按名称与描述排除虚拟网卡和无线网卡
fn is_physical_ethernet(name: &str, description: &str) -> bool {
    let d = format!("{name} {description}").to_ascii_lowercase();
    let virtual_markers = [
        "vmware",
        "vmnet",
        "virtual",
        "easytier",
        "hyper-v",
        "wsl",
        "tap-",
        "tap_",
        "loopback",
        "bluetooth",
        "wi-fi",
        "wifi",
        "802.11",
        "wireless",
        "vpn",
        "tidal",
        "cloudflare",
        "wan miniport",
        "microsoft km",
    ];
    if virtual_markers.iter().any(|m| d.contains(m)) {
        return false;
    }
    // 物理以太网的常见指纹：以太网中文名、ethernet、realtek/intel 有线芯片
    d.contains("以太网")
        || d.contains("ethernet")
        || d.contains("2.5gbe")
        || d.contains("gigabit")
        || d.contains("gbe family")
}

/// 给第一个物理以太网口配置直连地址（不设网关、网络位置设"专用"）。
///
/// 若该网口已有同网段地址则视为已配置，直接返回，不盲目覆盖
///
/// # Errors
///
/// - [`Error::NicNotFound`]：找不到物理以太网口
/// - [`Error::Shell`]：PowerShell 执行失败（常见原因：未以管理员运行）
pub fn setup_direct_link(host_octet: u8) -> Result<String> {
    let Some(nic) = list_nics().into_iter().find(|n| n.is_physical) else {
        return Err(Error::NicNotFound);
    };
    let ip = format!("{LINK_SUBNET_PREFIX}.{host_octet}");
    if let Some(cur) = &nic.current_ipv4 {
        if cur.starts_with(LINK_SUBNET_PREFIX) {
            return Ok(cur.clone());
        }
    }
    let safe_name = nic.name.replace('\'', "''");
    let script = format!(
        "New-NetIPAddress -InterfaceAlias '{safe_name}' -IPAddress {ip} -PrefixLength 24 -ErrorAction Stop; \
         Set-NetConnectionProfile -InterfaceAlias '{safe_name}' -NetworkCategory Private -ErrorAction SilentlyContinue"
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .map_err(|e| Error::Shell(e.to_string()))?;
    if !out.status.success() {
        return Err(Error::Shell(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ));
    }
    Ok(ip)
}

/// 还原直连配置：删除本工具配置的 192.168.88.x 地址并恢复 DHCP
///
/// # Errors
///
/// [`Error::Shell`]：PowerShell 执行失败（常见原因：未以管理员运行）。
pub fn revert_direct_link(host_octet: u8) -> Result<()> {
    let ip = format!("{LINK_SUBNET_PREFIX}.{host_octet}");
    let mut last_err = String::new();
    let mut touched = 0;
    for nic in list_nics() {
        if !nic.is_physical {
            continue;
        }
        touched += 1;
        let safe_name = nic.name.replace('\'', "''");
        let script = format!(
            "Remove-NetIPAddress -InterfaceAlias '{safe_name}' -IPAddress {ip} -Confirm:$false -ErrorAction SilentlyContinue; \
             Set-NetIPInterface -InterfaceAlias '{safe_name}' -Dhcp Enabled -ErrorAction SilentlyContinue"
        );
        match std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output()
        {
            Ok(o) if o.status.success() => {}
            Ok(o) => last_err = String::from_utf8_lossy(&o.stderr).trim().into(),
            Err(e) => last_err = e.to_string(),
        }
    }
    if touched == 0 {
        return Err(Error::NicNotFound);
    }
    if last_err.is_empty() {
        Ok(())
    } else {
        Err(Error::Shell(last_err))
    }
}

/// 对端地址（同网段另一台机器约定用对方 octet）。
#[must_use]
pub fn peer_ip(host_octet: u8) -> String {
    format!("{LINK_SUBNET_PREFIX}.{host_octet}")
}

/// 目标盘剩余空间（字节）。`drive` 接受 "C" 或 "C:\" 形式。
///
/// # Errors
///
/// 盘符无效或查询失败时返回 [`Error::Io`]。
pub fn free_bytes(drive: &str) -> Result<u64> {
    let root = if drive.len() == 1 {
        format!("{drive}:\\")
    } else {
        drive.to_owned()
    };
    fs4::free_space(std::path::Path::new(&root))
        .map_err(|e| Error::Io(format!("查询 {root} 可用空间失败: {e}")))
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn physical_ethernet_detection() {
        assert!(is_physical_ethernet(
            "以太网",
            "Intel(R) Ethernet Controller I225-V"
        ));
        assert!(is_physical_ethernet(
            "Ethernet",
            "Intel(R) Ethernet Connection I219-V"
        ));
        assert!(!is_physical_ethernet(
            "VMware Network Adapter VMnet8",
            "VMware Virtual Ethernet for VMnet8"
        ));
        assert!(!is_physical_ethernet("et_10_sd4s", "EasyTier Virtual NIC"));
        assert!(!is_physical_ethernet(
            "WLAN",
            "Intel(R) Wi-Fi 6 AX201 160MHz"
        ));
        assert!(!is_physical_ethernet(
            "本地连接* 3",
            "Microsoft Wi-Fi Direct Virtual Adapter"
        ));
    }

    #[test]
    fn peer_ip_same_subnet() {
        assert_eq!(peer_ip(2), "192.168.88.2");
    }

    #[test]
    fn nic_line_splitting() {
        assert_eq!(
            split_nic_line("以太网|Gigabit Ethernet Controller|192.168.88.1"),
            Some((
                "以太网".to_owned(),
                "Gigabit Ethernet Controller".to_owned(),
                Some("192.168.88.1".to_owned())
            ))
        );
        // 无 IP 的网口
        assert_eq!(
            split_nic_line("本地连接* 10|Microsoft Wi-Fi Direct Virtual Adapter #2|"),
            Some((
                "本地连接* 10".to_owned(),
                "Microsoft Wi-Fi Direct Virtual Adapter #2".to_owned(),
                None
            ))
        );
        // 描述里含 | 时 IP 仍是最后一段
        assert_eq!(
            split_nic_line("a|desc|with|pipe|10.0.0.1").map(|(_, _, ip)| ip),
            Some(Some("10.0.0.1".to_owned()))
        );
        // 行数不足
        assert_eq!(split_nic_line("只有一段"), None);
    }

    #[test]
    fn list_nics_runs_without_panic() {
        // 不做内容断言（CI 无网卡环境差异大），只验证结构完整
        for nic in list_nics() {
            assert!(!nic.name.is_empty());
        }
    }

    #[test]
    fn free_bytes_of_system_drive_is_positive() {
        #[cfg(windows)]
        let drive = "C";
        #[cfg(not(windows))]
        let drive = "/";
        let free = free_bytes(drive).expect("system drive must be queryable");
        assert!(free > 0);
    }
}
