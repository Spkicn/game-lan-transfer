//! 直连自举：识别物理以太网口、配置不设网关的静态 IP、还原网络配置
//!
//! 改网络配置的操作走 PowerShell，UAC 由调用方负责；只读查询也用 PowerShell 一次取齐
//! 只对当前未配置的网口动作，已有可用 IPv4 的网口跳过

use crate::{Error, Result};

/// 直连网段 192.168.88.0/24
pub const LINK_SUBNET_PREFIX: &str = "192.168.88";

/// 一个候选网口的描述
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nic {
    /// Windows 网络连接名，如 "以太网"
    pub name: String,
    /// 网卡硬件描述，取自 Windows `InterfaceDescription`
    pub description: String,
    /// 是否为物理以太网口，虚拟网卡与 WLAN 已排除
    pub is_physical: bool,
    /// 现有 IPv4 地址，`LinkLocal` 169.254 除外；None 表示未配置
    pub current_ipv4: Option<String>,
    /// 该网口上的全部 IPv4 地址，判断是否已配置过直连地址时要看全
    pub all_ipv4: Vec<String>,
}

impl Nic {
    /// 该网口是否已经有这个地址
    #[must_use]
    pub fn has_address(&self, ip: &str) -> bool {
        self.all_ipv4.iter().any(|value| value == ip)
    }
}

/// 一次直连配置的结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSetup {
    /// 被配置的网口名
    pub nic_name: String,
    /// 配置后的地址
    pub ip: String,
    /// 之前就已经配好，本次没有改动
    pub already: bool,
}

/// 执行一段 PowerShell，输出按 UTF-8 解，失败时报可读的原因
///
/// # Errors
///
/// PowerShell 无法启动或脚本以非零状态结束时返回 [`Error::Shell`]
pub fn run_powershell(script: &str) -> Result<String> {
    run_powershell_in(script, &[])
}

/// 执行一段 PowerShell，可追加参数
///
/// # Errors
///
/// PowerShell 无法启动或脚本以非零状态结束时返回 [`Error::Shell`]
pub fn run_powershell_in(script: &str, args: &[&str]) -> Result<String> {
    let full = format!("[Console]::OutputEncoding=[Text.Encoding]::UTF8; {script}");
    let output = crate::shell::hidden_command("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &full])
        .args(args)
        .output()
        .map_err(|err| Error::Shell(err.to_string()))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let raw = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr.into_owned()
    };
    Err(Error::Shell(friendly_shell_error(&raw)))
}

/// 把 PowerShell 的异常输出压成一句能看懂的话
///
/// 未提权时 Windows 返回 `System Error 5`，异常正文后面还跟着 `CategoryInfo` 一类定位信息
#[must_use]
pub fn friendly_shell_error(raw: &str) -> String {
    let lowered = raw.to_lowercase();
    if lowered.contains("system error 5")
        || lowered.contains("permissiondenied")
        || lowered.contains("access is denied")
        || raw.contains("拒绝访问")
    {
        return "需要管理员权限：关闭本窗口，右键 GameLift 选择「以管理员身份运行」后再试"
            .to_owned();
    }
    if lowered.contains("cancelled by the user")
        || lowered.contains("canceled by the user")
        || raw.contains("用户取消")
        || raw.contains("已被用户取消")
    {
        return "管理员权限请求被取消：想改网络设置时再点一次即可".to_owned();
    }
    let text = raw
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with('+')
                && !line.starts_with('~')
                && !line.starts_with("At line:")
                && !line.starts_with("CategoryInfo")
                && !line.starts_with("FullyQualifiedErrorId")
        })
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "PowerShell 执行失败".to_owned();
    }
    trimmed.chars().take(300).collect()
}

/// 提权探测脚本，必须写成一整行
///
/// 用续行拼接会在 `)` 与 `.IsInRole` 之间留下空格，PowerShell 会直接解析失败，
/// 于是任何权限都被判成未提权
const ELEVATION_PROBE: &str = "(([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator))";

/// 当前进程是否以管理员身份运行，结果缓存
///
/// 探测失败时返回 `None`，调用方不要据此拦截操作
#[must_use]
pub fn elevation_state() -> Option<bool> {
    static STATE: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *STATE.get_or_init(|| {
        run_powershell(ELEVATION_PROBE)
            .ok()
            .map(|text| text.trim().eq_ignore_ascii_case("true"))
    })
}

/// 是否以管理员身份运行
///
/// 探测不出来时按已提权处理：真正的失败会带自己的原因冒出来，比误判拦下操作好
#[must_use]
pub fn is_elevated() -> bool {
    elevation_state() != Some(false)
}

/// 提示用户以管理员身份重新启动本程序
pub const ELEVATION_HINT: &str =
    "需要管理员权限：关闭本窗口，右键 GameLift 选择「以管理员身份运行」后再试";

/// 罗列所有网口，单次 PowerShell 调用取齐名称、描述与全部 IPv4
/// 输出编码显式设为 UTF-8，避免中文 Windows 控制台乱码
#[must_use]
pub fn list_nics() -> Vec<Nic> {
    let ps = "[Console]::OutputEncoding=[Text.Encoding]::UTF8; \
              Get-NetAdapter | ForEach-Object { \
                $n=$_.Name; \
                $ips=(Get-NetIPAddress -InterfaceAlias $n -AddressFamily IPv4 -ErrorAction SilentlyContinue \
                     | Where-Object { $_.IPAddress -notlike '169.254*' -and $_.IPAddress -notlike '127.*' } \
                     | ForEach-Object { $_.IPAddress }) -join ','; \
                '{0}|{1}|{2}' -f $n, $_.InterfaceDescription, $ips \
              }";
    let output = crate::shell::hidden_command("powershell")
        .args(["-NoProfile", "-Command", ps])
        .output();
    let Ok(out) = output else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut nics = Vec::new();
    for line in text.lines() {
        let Some((name, desc, addresses)) = split_nic_line(line) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let current_ipv4 = addresses.first().cloned();
        nics.push(Nic {
            current_ipv4,
            all_ipv4: addresses,
            is_physical: is_physical_ethernet(&name, &desc),
            description: desc,
            name,
        });
    }
    nics
}

/// 拆 `名称|描述|地址表` 行，最后一段是逗号分隔的 IPv4 列表，允许为空
/// 描述若含 `|`，靠 rsplitn 保证地址仍取最后一段
fn split_nic_line(line: &str) -> Option<(String, String, Vec<String>)> {
    let (head, ips_raw) = line.rsplit_once('|')?;
    let (name_raw, desc_raw) = head.split_once('|')?;
    let addresses = ips_raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect();
    Some((
        name_raw.trim().to_owned(),
        desc_raw.trim().to_owned(),
        addresses,
    ))
}

/// 物理以太网口判定：名称或描述命中虚拟网卡、无线标记即排除
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

/// 给第一个物理以太网口配置直连地址，不设网关并把网络位置设为专用
///
/// 网口上已经有这个地址时视为已配置并直接返回
///
/// # Errors
///
/// - [`Error::NicNotFound`]：找不到物理以太网口
/// - [`Error::Shell`]：未以管理员运行，或 PowerShell 执行失败
pub fn configure_direct_link(host_octet: u8) -> Result<LinkSetup> {
    let Some(nic) = list_nics().into_iter().find(|n| n.is_physical) else {
        return Err(Error::NicNotFound);
    };
    let ip = format!("{LINK_SUBNET_PREFIX}.{host_octet}");
    // 网卡上任何一个地址命中都算已配置，只看第一个地址会重复下发并撞上「已存在」
    if nic.has_address(&ip) {
        return Ok(LinkSetup {
            nic_name: nic.name,
            ip,
            already: true,
        });
    }
    if !is_elevated() {
        return Err(Error::Shell(ELEVATION_HINT.to_owned()));
    }
    let safe_name = nic.name.replace('\'', "''");
    // 先查再补，避免网络波动后重试时报「地址已存在」
    let script = format!(
        "$existing = Get-NetIPAddress -InterfaceAlias '{safe_name}' -AddressFamily IPv4 -ErrorAction SilentlyContinue \
           | Where-Object {{ $_.IPAddress -eq '{ip}' }}; \
         if (-not $existing) {{ \
           New-NetIPAddress -InterfaceAlias '{safe_name}' -IPAddress {ip} -PrefixLength 24 -ErrorAction Stop \
         }}; \
         Set-NetConnectionProfile -InterfaceAlias '{safe_name}' -NetworkCategory Private -ErrorAction SilentlyContinue"
    );
    run_powershell(&script)?;
    // 读回确认，没配上就报出来，不要假装成功
    let applied = list_nics()
        .into_iter()
        .any(|candidate| candidate.name == nic.name && candidate.has_address(&ip));
    if !applied {
        return Err(Error::Shell(format!(
            "地址 {ip} 没有落到网口 {} 上，请在「网络和 Internet」里手动确认该网口的 IPv4 设置",
            nic.name
        )));
    }
    Ok(LinkSetup {
        nic_name: nic.name,
        ip,
        already: false,
    })
}

/// 配置直连地址并返回地址文本
///
/// # Errors
///
/// 同 [`configure_direct_link`]
pub fn setup_direct_link(host_octet: u8) -> Result<String> {
    configure_direct_link(host_octet).map(|setup| setup.ip)
}

/// 还原直连配置：删本工具配置的 192.168.88.x 地址并恢复 DHCP
///
/// # Errors
///
/// [`Error::Shell`]：PowerShell 执行失败，通常是未以管理员运行
pub fn revert_direct_link(host_octet: u8) -> Result<()> {
    let ip = format!("{LINK_SUBNET_PREFIX}.{host_octet}");
    let mut last_err = String::new();
    let mut touched = 0;
    if !is_elevated() {
        return Err(Error::Shell(ELEVATION_HINT.to_owned()));
    }
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
        if let Err(err) = run_powershell(&script) {
            last_err = err.to_string();
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

/// 对端地址，约定同网段另一台机器使用对方 octet
#[must_use]
pub fn peer_ip(host_octet: u8) -> String {
    format!("{LINK_SUBNET_PREFIX}.{host_octet}")
}

/// 可用于局域网传输的本机地址
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAddress {
    /// 网卡名称
    pub nic_name: String,
    /// IPv4 地址
    pub ip: String,
    /// 是否为物理以太网口
    pub is_physical: bool,
}

/// 选一个可用于局域网传输的本机地址
///
/// 优先物理以太网口上已配置的地址，其次是任意网卡上已配置的地址，
/// 这样只连 Wi-Fi 的机器也能直接使用，不必先插网线配静态地址
#[must_use]
pub fn preferred_ipv4(nics: &[Nic]) -> Option<LocalAddress> {
    let mut fallback: Option<LocalAddress> = None;
    for nic in nics {
        let Some(ip) = nic.current_ipv4.as_deref().filter(|ip| !ip.is_empty()) else {
            continue;
        };
        let candidate = LocalAddress {
            nic_name: nic.name.clone(),
            ip: ip.to_owned(),
            is_physical: nic.is_physical,
        };
        if nic.is_physical {
            return Some(candidate);
        }
        if fallback.is_none() {
            fallback = Some(candidate);
        }
    }
    fallback
}

/// 地址是否落在本工具配置的直连网段里
#[must_use]
pub fn is_direct_link_ip(ip: &str) -> bool {
    ip.starts_with(&format!("{LINK_SUBNET_PREFIX}."))
}

/// 直连地址当前是否可用，可用时返回 `None`
///
/// Windows 给地址维护一个状态：`Preferred` 可用，`Duplicate` 表示同网段里
/// 已有机器用同一个地址，多半是两台机器配成了同一个角色
#[must_use]
pub fn link_address_problem(ip: &str) -> Option<String> {
    let script = format!(
        "(Get-NetIPAddress -IPAddress {ip} -AddressFamily IPv4 -ErrorAction SilentlyContinue | Select-Object -First 1).AddressState"
    );
    let state = run_powershell(&script).ok()?;
    describe_address_state(state.trim())
}

/// 把地址状态翻成给用户看的话，可用时为 `None`
#[must_use]
pub fn describe_address_state(state: &str) -> Option<String> {
    match state.trim() {
        "" | "Preferred" => None,
        "Tentative" => Some("地址还在做重复检测，稍等几秒再点「找对端」".to_owned()),
        "Duplicate" => Some(
            "地址冲突：同一网段里已有机器在用这个地址，多半两台都选成了同一个角色。把其中一台改成另一种角色即可"
                .to_owned(),
        ),
        "Deprecated" | "Invalid" => Some("这个地址已失效，点「重新检测」，必要时重新选一次角色".to_owned()),
        other => Some(format!("地址状态异常：{other}")),
    }
}

/// 解析要使用的本机地址
///
/// 界面传进来的地址可能刚被改掉，先验它还在不在网卡上；不在就退回自动挑选
#[must_use]
pub fn resolve_local_ip(nics: &[Nic], explicit: Option<&str>) -> Option<String> {
    if let Some(wanted) = explicit.filter(|value| !value.is_empty()) {
        if wanted.parse::<std::net::IpAddr>().is_ok()
            && nics.iter().any(|nic| nic.has_address(wanted))
        {
            return Some(wanted.to_owned());
        }
    }
    preferred_ipv4(nics).map(|found| found.ip)
}

/// 目标盘剩余空间，单位字节；`drive` 接受 "C" 或 "C:\" 形式
///
/// # Errors
///
/// 盘符无效或查询失败时返回 [`Error::Io`]
pub fn free_bytes(drive: &str) -> Result<u64> {
    let root = if drive.len() == 1 {
        format!("{drive}:\\")
    } else {
        drive.to_owned()
    };
    fs4::free_space(std::path::Path::new(&root))
        .map_err(|e| Error::Io(format!("查询 {root} 可用空间失败: {e}")))
}

/// 路径所在卷的剩余空间，单位字节
///
/// # Errors
///
/// 路径不存在或查询失败时返回 [`Error::Io`]
pub fn free_bytes_at(path: &std::path::Path) -> Result<u64> {
    fs4::free_space(path)
        .map_err(|e| Error::Io(format!("查询 {} 可用空间失败: {e}", path.display())))
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn nic(name: &str, is_physical: bool, ip: Option<&str>) -> Nic {
        Nic {
            name: name.to_owned(),
            description: format!("{name} 描述"),
            is_physical,
            all_ipv4: ip.map(str::to_owned).into_iter().collect(),
            current_ipv4: ip.map(str::to_owned),
        }
    }

    #[test]
    fn nic_sees_every_address_on_the_adapter() {
        // 只看第一个地址会重复下发直连配置并撞上「地址已存在」
        let adapter = Nic {
            name: "以太网".to_owned(),
            description: "Realtek".to_owned(),
            is_physical: true,
            current_ipv4: Some("10.10.10.1".to_owned()),
            all_ipv4: vec!["10.10.10.1".to_owned(), "192.168.88.2".to_owned()],
        };
        assert!(adapter.has_address("192.168.88.2"));
        assert!(!adapter.has_address("192.168.88.1"));
    }

    #[test]
    fn parses_multiple_addresses_per_adapter() {
        let parsed = split_nic_line("以太网|Realtek GbE|10.10.10.1,192.168.88.2").expect("parse");
        assert_eq!(parsed.0, "以太网");
        assert_eq!(parsed.1, "Realtek GbE");
        assert_eq!(
            parsed.2,
            vec!["10.10.10.1".to_owned(), "192.168.88.2".to_owned()]
        );
        let empty = split_nic_line("WLAN|Intel AX201|").expect("parse");
        assert_eq!(empty.2, Vec::<String>::new());
    }

    #[test]
    fn permission_failure_becomes_an_actionable_hint() {
        let raw = "New-NetIPAddress : 取值为“1”的参数无效。\r\n\
                   + CategoryInfo          : PermissionDenied: (MSFT_NetIPAddress:ROOT/StandardCimv2/MSFT_NetIPAddress) [New-NetIPAddress], CimException\r\n\
                   + FullyQualifiedErrorId : Windows System Error 5,New-NetIPAddress\r\n";
        let message = friendly_shell_error(raw);
        assert!(message.contains("管理员权限"), "got {message}");
        assert!(!message.contains("CategoryInfo"), "got {message}");
    }

    #[test]
    fn other_failures_keep_the_first_line_and_drop_noise() {
        let raw = "Remove-NetIPAddress : 找不到指定的接口。\r\n\
                   + CategoryInfo          : ObjectNotFound: (MSFT_NetIPAddress:ROOT/StandardCimv2/MSFT_NetIPAddress) [Remove-NetIPAddress], CimException\r\n\
                   + FullyQualifiedErrorId : Windows System Error 1168,Remove-NetIPAddress\r\n";
        let message = friendly_shell_error(raw);
        assert!(message.contains("找不到指定的接口"), "got {message}");
        assert!(!message.contains("FullyQualifiedErrorId"), "got {message}");
        assert!(message.len() <= 300);
    }

    #[test]
    fn address_state_turns_into_actionable_text() {
        assert_eq!(describe_address_state("Preferred"), None);
        assert_eq!(describe_address_state(""), None);
        let duplicate = describe_address_state("Duplicate").expect("duplicate");
        assert!(duplicate.contains("同一个角色"), "got {duplicate}");
        let tentative = describe_address_state("Tentative").expect("tentative");
        assert!(tentative.contains("重复检测"), "got {tentative}");
        assert!(describe_address_state("什么也不是").is_some());
    }

    #[test]
    fn stale_explicit_address_falls_back_to_detection() {
        let nics = vec![nic("以太网", true, Some("192.168.88.2"))];
        assert_eq!(
            resolve_local_ip(&nics, Some("192.168.88.2")).as_deref(),
            Some("192.168.88.2")
        );
        // 界面上的地址已经被改掉了，不能拿它去 bind
        assert_eq!(
            resolve_local_ip(&nics, Some("192.168.88.1")).as_deref(),
            Some("192.168.88.2")
        );
        assert_eq!(
            resolve_local_ip(&nics, None).as_deref(),
            Some("192.168.88.2")
        );
    }

    #[test]
    fn no_address_at_all_returns_none() {
        let nics = vec![nic("WLAN", false, None)];
        assert_eq!(resolve_local_ip(&nics, Some("192.168.88.1")), None);
    }

    #[test]
    fn elevation_probe_script_stays_on_one_line() {
        // 续行拼接会插入空格，PowerShell 解析失败后所有人都被判成未提权
        assert!(ELEVATION_PROBE.contains(").IsInRole("), "{ELEVATION_PROBE}");
        assert!(!ELEVATION_PROBE.contains(") ."), "{ELEVATION_PROBE}");
        assert!(!ELEVATION_PROBE.contains('\n'), "{ELEVATION_PROBE}");
    }

    #[test]
    fn elevation_probe_answers_on_this_machine() {
        // 探测必须真的给出答案，取不到答案说明脚本本身有问题
        assert!(
            elevation_state().is_some(),
            "提权探测没有返回结果，说明探测脚本执行失败"
        );
    }

    #[test]
    fn cancelled_elevation_is_not_reported_as_a_failure() {
        let raw = "Start-Process : 此操作已被用户取消。\r\n\
                   + CategoryInfo          : InvalidOperation: (:) [Start-Process], InvalidOperationException\r\n";
        let message = friendly_shell_error(raw);
        assert!(message.contains("被取消"), "got {message}");
    }

    #[test]
    fn empty_output_still_gives_a_message() {
        assert_eq!(friendly_shell_error("  \r\n \r\n"), "PowerShell 执行失败");
    }

    #[test]
    fn preferred_ipv4_takes_physical_first() {
        let nics = vec![
            nic("WLAN", false, Some("10.0.0.5")),
            nic("以太网", true, Some("192.168.88.1")),
        ];
        let picked = preferred_ipv4(&nics).expect("应选中物理口");
        assert_eq!(picked.ip, "192.168.88.1");
        assert!(picked.is_physical);
        assert!(is_direct_link_ip(&picked.ip));
    }

    #[test]
    fn preferred_ipv4_falls_back_to_other_nic() {
        let nics = vec![
            nic("以太网", true, None),
            nic("WLAN", false, Some("172.20.16.119")),
        ];
        let picked = preferred_ipv4(&nics).expect("应回退到无线网卡");
        assert_eq!(picked.ip, "172.20.16.119");
        assert!(!picked.is_physical);
        assert!(!is_direct_link_ip(&picked.ip));
    }

    #[test]
    fn preferred_ipv4_none_without_address() {
        let nics = vec![nic("以太网", true, None), nic("WLAN", false, Some(""))];
        assert!(preferred_ipv4(&nics).is_none());
    }

    #[test]
    fn physical_ethernet_detection() {
        assert!(is_physical_ethernet(
            "以太网",
            "Gigabit Ethernet Controller"
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
                vec!["192.168.88.1".to_owned()]
            ))
        );
        // 无 IP 的网口
        assert_eq!(
            split_nic_line("本地连接* 10|Microsoft Wi-Fi Direct Virtual Adapter #2|"),
            Some((
                "本地连接* 10".to_owned(),
                "Microsoft Wi-Fi Direct Virtual Adapter #2".to_owned(),
                Vec::new()
            ))
        );
        // 描述里含 | 时地址仍是最后一段
        assert_eq!(
            split_nic_line("a|desc|with|pipe|10.0.0.1").map(|(_, _, ips)| ips),
            Some(vec!["10.0.0.1".to_owned()])
        );
        // 行数不足
        assert_eq!(split_nic_line("只有一段"), None);
    }

    #[test]
    fn list_nics_runs_without_panic() {
        // CI 环境网卡差异大，不做内容断言，只验证结构完整
        for nic in list_nics() {
            assert_ne!(nic.name, "");
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
