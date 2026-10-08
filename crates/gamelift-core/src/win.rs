#![allow(unsafe_code)]

//! Windows 原生能力
//!
//! 这些以前靠内联 PowerShell 脚本：脚本没有编译期检查，编码、转义、续行都能出事
//!
//! 调 Win32 必须用 unsafe，全项目只在本模块放开，边界写在这里，别处仍禁

use std::ffi::OsString;
use std::net::Ipv4Addr;
use std::os::windows::ffi::OsStringExt;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::NetworkManagement::IpHelper::{
    CreateUnicastIpAddressEntry, DeleteUnicastIpAddressEntry, GetAdaptersAddresses,
    InitializeUnicastIpAddressEntry, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
    GAA_FLAG_SKIP_MULTICAST, GET_ADAPTERS_ADDRESSES_FLAGS, IP_ADAPTER_ADDRESSES_LH,
    MIB_UNICASTIPADDRESS_ROW,
};
use windows::Win32::Networking::WinSock::{AF_INET, AF_UNSPEC, SOCKADDR_IN};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// 地址状态里「可用」的取值，与 Windows 的 `NL_DAD_STATE` 一致
pub const DAD_PREFERRED: i32 = 4;

/// 一块网卡与它上面的 IPv4 地址
#[derive(Debug, Clone)]
pub struct Adapter {
    /// 网卡名称，例如「以太网」
    pub name: String,
    /// 网卡描述
    pub description: String,
    /// 是否是以太网或无线这类物理网卡
    pub is_physical: bool,
    /// 接口 LUID，写静态地址时用它指定网卡
    pub luid: u64,
    /// 地址与它的重复检测状态
    pub ipv4: Vec<(Ipv4Addr, i32)>,
}

/// 当前进程是否以管理员身份运行
#[must_use]
pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY,
            std::ptr::addr_of_mut!(token),
        )
        .is_err()
        {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0_u32;
        let size = u32::try_from(std::mem::size_of::<TOKEN_ELEVATION>()).unwrap_or(0);
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(std::ptr::addr_of_mut!(elevation).cast()),
            size,
            std::ptr::addr_of_mut!(returned),
        );
        let _ = CloseHandle(token);
        ok.is_ok() && elevation.TokenIsElevated != 0
    }
}

/// 罗列所有网卡及其 IPv4 地址，取不到时返回空表
#[must_use]
#[allow(clippy::cast_ptr_alignment)]
pub fn adapters() -> Vec<Adapter> {
    const ERROR_BUFFER_OVERFLOW: u32 = 111;
    let flags = GET_ADAPTERS_ADDRESSES_FLAGS(
        GAA_FLAG_SKIP_ANYCAST.0 | GAA_FLAG_SKIP_MULTICAST.0 | GAA_FLAG_SKIP_DNS_SERVER.0,
    );
    let mut size = 16 * 1024_u32;
    let mut buffer: Vec<u8> = Vec::new();
    loop {
        buffer.clear();
        buffer.resize(size as usize, 0);
        let ret = unsafe {
            GetAdaptersAddresses(
                u32::from(AF_UNSPEC.0),
                flags,
                None,
                Some(buffer.as_mut_ptr().cast()),
                std::ptr::addr_of_mut!(size),
            )
        };
        if ret == 0 {
            break;
        }
        if ret != ERROR_BUFFER_OVERFLOW {
            return Vec::new();
        }
    }

    let mut out = Vec::new();
    unsafe {
        let mut current = buffer.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        while let Some(adapter) = current.as_ref() {
            let name = wide_to_string(adapter.FriendlyName);
            let description = wide_to_string(adapter.Description);
            let mut ipv4 = Vec::new();
            let mut unicast = adapter.FirstUnicastAddress;
            while let Some(entry) = unicast.as_ref() {
                let raw = entry.Address.lpSockaddr;
                if !raw.is_null() && (*raw).sa_family == AF_INET {
                    // 系统给的缓冲区按 8 字节对齐，SOCKADDR 与 SOCKADDR_IN 等长，族已确认
                    let v4 = &*raw.cast::<SOCKADDR_IN>();
                    ipv4.push((
                        Ipv4Addr::from(v4.sin_addr.S_un.S_addr.to_ne_bytes()),
                        entry.DadState.0,
                    ));
                }
                unicast = entry.Next;
            }
            // 6 是以太网，71 是无线
            out.push(Adapter {
                name,
                description,
                is_physical: adapter.IfType == 6 || adapter.IfType == 71,
                luid: adapter.Luid.Value,
                ipv4,
            });
            current = adapter.Next;
        }
    }
    out
}

/// 某个地址当前的重复检测状态
#[must_use]
pub fn address_state(ip: &str) -> Option<i32> {
    let wanted: Ipv4Addr = ip.parse().ok()?;
    adapters()
        .into_iter()
        .flat_map(|adapter| adapter.ipv4)
        .find(|(address, _)| *address == wanted)
        .map(|(_, state)| state)
}

/// 把宽字符串指针读成 Rust 字符串
fn wide_to_string(ptr: windows::core::PWSTR) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0_usize;
    unsafe {
        while *ptr.0.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(ptr.0, len);
        OsString::from_wide(slice).to_string_lossy().into_owned()
    }
}

/// 以管理员身份重启指定程序
///
/// 走 `ShellExecuteW` 的 runas 动词，不再拼 PowerShell 的 Start-Process
///
/// # Errors
///
/// 用户在 UAC 上取消或系统拒绝启动时返回说明
pub fn relaunch_as_admin(exe: &std::path::Path, args: &[&str]) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide = |text: &str| -> Vec<u16> {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    };
    let file = wide(&exe.to_string_lossy());
    let parameters = wide(&args.join(" "));
    let verb = wide("runas");
    let started = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR(parameters.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // 返回值大于 32 才算成功，5 是用户按了「否」
    let code = started.0 as usize;
    if code > 32 {
        return Ok(());
    }
    if code == 5 {
        return Err("权限请求被取消：想改网络设置时再点一次即可".to_owned());
    }
    Err(format!("以管理员身份启动失败，错误码 {code}"))
}

/// 给指定网卡加一个静态 IPv4 地址
///
/// # Errors
///
/// 网卡不存在、未以管理员运行或地址已存在时返回可读说明
pub fn add_static_ipv4(
    adapter_name: &str,
    address: Ipv4Addr,
    prefix_len: u8,
) -> Result<(), String> {
    let row = address_row(adapter_name, address, prefix_len)?;
    let code = unsafe { CreateUnicastIpAddressEntry(std::ptr::addr_of!(row)) }.0;
    match code {
        0 => Ok(()),
        183 => Err(format!("地址 {address} 已经在这块网卡上，无需重复配置")),
        5 => Err("需要管理员权限：以管理员身份运行后再配直连".to_owned()),
        other => Err(format!("添加地址 {address} 失败，错误码 {other}")),
    }
}

/// 从指定网卡上摘掉一个 IPv4 地址
///
/// # Errors
///
/// 网卡不存在、未以管理员运行或地址本来就不在时返回可读说明
pub fn remove_static_ipv4(
    adapter_name: &str,
    address: Ipv4Addr,
    prefix_len: u8,
) -> Result<(), String> {
    let row = address_row(adapter_name, address, prefix_len)?;
    let code = unsafe { DeleteUnicastIpAddressEntry(std::ptr::addr_of!(row)) }.0;
    match code {
        0 | 1168 => Ok(()), // 1168 是本来就没有，按已达成处理
        5 => Err("需要管理员权限：以管理员身份运行后再还原网络设置".to_owned()),
        other => Err(format!("移除地址 {address} 失败，错误码 {other}")),
    }
}

/// 组装一条静态地址记录，Win32 靠它定位网卡与地址
#[allow(clippy::cast_ptr_alignment)]
fn address_row(
    adapter_name: &str,
    address: Ipv4Addr,
    prefix_len: u8,
) -> Result<MIB_UNICASTIPADDRESS_ROW, String> {
    let adapter = adapters()
        .into_iter()
        .find(|item| item.name == adapter_name)
        .ok_or_else(|| format!("没有找到网卡 {adapter_name}"))?;
    if adapter.luid == 0 {
        return Err(format!("网卡 {adapter_name} 没有可用的接口标识"));
    }
    let mut row = MIB_UNICASTIPADDRESS_ROW::default();
    unsafe {
        InitializeUnicastIpAddressEntry(std::ptr::addr_of_mut!(row));
        row.InterfaceLuid.Value = adapter.luid;
        row.OnLinkPrefixLength = prefix_len;
        row.Address.si_family = AF_INET;
        row.Address.Ipv4.sin_family = AF_INET;
        row.Address.Ipv4.sin_addr.S_un.S_addr = u32::from_ne_bytes(address.octets());
    }
    Ok(row)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn adapters_are_visible_on_this_machine() {
        // 原生枚举取不到网卡的话，网络功能整条都不可用
        let list = adapters();
        assert!(!list.is_empty(), "没有枚举到任何网卡");
        assert!(
            list.iter().any(|adapter| !adapter.name.is_empty()),
            "网卡名称为空"
        );
    }

    #[test]
    fn elevation_answers_without_a_script() {
        // 结果随运行环境而定，这里只验证两次调用一致而不是报错
        let elevated = is_elevated();
        assert_eq!(elevated, is_elevated(), "两次调用结果必须一致");
    }

    #[test]
    fn adapters_carry_an_interface_luid() {
        // 写静态地址要用 LUID 指定网卡，全零说明没取到
        let list = adapters();
        assert!(
            list.iter().any(|adapter| adapter.luid != 0),
            "所有网卡的 LUID 都是零：{list:?}"
        );
    }

    #[test]
    fn adding_an_address_without_admin_is_refused_clearly() {
        // 本会话不是管理员，这条正好覆盖"未提权时报错清晰"这一半
        if is_elevated() {
            return;
        }
        let Some(adapter) = adapters().into_iter().find(|item| item.is_physical) else {
            return;
        };
        // 192.0.2.0/24 是文档专用网段，真机上不会被占用
        let address: Ipv4Addr = "192.0.2.7".parse().expect("ip");
        let outcome = add_static_ipv4(&adapter.name, address, 24);
        let Err(message) = outcome else {
            panic!("未提权却配置成功，说明这条路径绕过了权限检查");
        };
        assert!(message.contains("管理员"), "错误信息该说明权限：{message}");
    }

    #[test]
    fn unknown_adapter_is_reported_by_name() {
        let outcome = add_static_ipv4("gamelift-no-such-nic", "192.0.2.8".parse().expect("ip"), 24);
        let Err(message) = outcome else {
            panic!("不存在的网卡不该配置成功");
        };
        assert!(message.contains("没有找到"), "got {message}");
    }

    #[test]
    fn unknown_address_has_no_state() {
        assert_eq!(address_state("203.0.113.7"), None);
    }
}
