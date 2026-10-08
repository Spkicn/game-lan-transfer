//! 注册表读取：真机读 Windows 注册表，测试喂固定数据
//!
//! 战网、EA、Ubisoft、GOG 都把安装位置记在注册表里，
//! 这里把读取抽成 trait，适配器就能在没有装这些启动器的机器上被测到

use std::collections::BTreeMap;

/// 一个键下面的值表，键名到值
pub type ValueMap = BTreeMap<String, String>;

/// 注册表读取接口
pub trait RegistryRead {
    /// 列出某个键下的子键名，键不存在时返回空
    fn subkeys(&self, key: &str) -> Vec<String>;

    /// 读取某个键下的所有值，键不存在时返回空
    fn values(&self, key: &str) -> ValueMap;
}

/// 真机实现，用 winreg 直接读注册表，不再起 `reg query`
#[cfg(windows)]
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsRegistry;

#[cfg(windows)]
impl RegistryRead for WindowsRegistry {
    fn subkeys(&self, key: &str) -> Vec<String> {
        let Some(opened) = open(key) else {
            return Vec::new();
        };
        let mut names: Vec<String> = opened.enum_keys().flatten().collect();
        names.sort();
        names
    }

    fn values(&self, key: &str) -> ValueMap {
        let Some(opened) = open(key) else {
            return ValueMap::new();
        };
        let mut out = ValueMap::new();
        for (name, value) in opened.enum_values().flatten() {
            out.insert(name, format_value(&value));
        }
        out
    }
}

/// 非 Windows 平台没有注册表
#[cfg(not(windows))]
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsRegistry;

#[cfg(not(windows))]
impl RegistryRead for WindowsRegistry {
    fn subkeys(&self, _key: &str) -> Vec<String> {
        Vec::new()
    }

    fn values(&self, _key: &str) -> ValueMap {
        ValueMap::new()
    }
}

/// 测试用实现，直接给出键到值的映射
#[derive(Debug, Clone, Default)]
pub struct MapRegistry {
    keys: BTreeMap<String, ValueMap>,
}

impl MapRegistry {
    /// 用若干键值表构造
    #[must_use]
    pub fn new(entries: impl IntoIterator<Item = (String, ValueMap)>) -> Self {
        Self {
            keys: entries.into_iter().collect(),
        }
    }
}

impl RegistryRead for MapRegistry {
    fn subkeys(&self, key: &str) -> Vec<String> {
        let prefix = format!("{key}\\");
        self.keys
            .keys()
            .filter_map(|candidate| candidate.strip_prefix(&prefix))
            .filter(|rest| !rest.is_empty() && !rest.contains('\\'))
            .map(str::to_owned)
            .collect()
    }

    fn values(&self, key: &str) -> ValueMap {
        self.keys.get(key).cloned().unwrap_or_default()
    }
}

/// 打开 `HIVE\子键` 形式的键，短写与长写都认
#[cfg(windows)]
fn open(key: &str) -> Option<winreg::RegKey> {
    use winreg::enums::{
        HKEY_CLASSES_ROOT, HKEY_CURRENT_CONFIG, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS,
        KEY_READ,
    };
    use winreg::RegKey;

    let (hive, rest) = key.split_once('\\').unwrap_or((key, ""));
    let root = match hive.to_ascii_uppercase().as_str() {
        "HKLM" | "HKEY_LOCAL_MACHINE" => RegKey::predef(HKEY_LOCAL_MACHINE),
        "HKCU" | "HKEY_CURRENT_USER" => RegKey::predef(HKEY_CURRENT_USER),
        "HKCR" | "HKEY_CLASSES_ROOT" => RegKey::predef(HKEY_CLASSES_ROOT),
        "HKU" | "HKEY_USERS" => RegKey::predef(HKEY_USERS),
        "HKCC" | "HKEY_CURRENT_CONFIG" => RegKey::predef(HKEY_CURRENT_CONFIG),
        _ => return None,
    };
    if rest.is_empty() {
        return Some(root);
    }
    root.open_subkey_with_flags(rest, KEY_READ).ok()
}

/// 把注册表值转成字符串，与 `reg query` 的显示保持一致
#[cfg(windows)]
fn format_value(value: &winreg::RegValue) -> String {
    use std::fmt::Write as _;
    use winreg::enums::RegType;
    match value.vtype {
        RegType::REG_SZ | RegType::REG_EXPAND_SZ => decode_wide(&value.bytes),
        RegType::REG_MULTI_SZ => wide_units(&value.bytes)
            .split(|unit| *unit == 0)
            .filter(|part| !part.is_empty())
            .map(String::from_utf16_lossy)
            .collect::<Vec<String>>()
            .join(" / "),
        RegType::REG_DWORD => u32::from_le_bytes(
            value
                .bytes
                .get(..4)
                .and_then(|slice| slice.try_into().ok())
                .unwrap_or([0; 4]),
        )
        .to_string(),
        RegType::REG_QWORD => u64::from_le_bytes(
            value
                .bytes
                .get(..8)
                .and_then(|slice| slice.try_into().ok())
                .unwrap_or([0; 8]),
        )
        .to_string(),
        RegType::REG_BINARY => {
            let mut hex = String::with_capacity(value.bytes.len() * 2);
            for byte in &value.bytes {
                let _ = write!(hex, "{byte:02x}");
            }
            hex
        }
        _ => String::new(),
    }
}

/// UTF-16 小端字节流转 u16 序列
#[cfg(windows)]
fn wide_units(bytes: &[u8]) -> Vec<u16> {
    let mut units = Vec::with_capacity(bytes.len() / 2);
    let mut index = 0;
    while index + 1 < bytes.len() {
        units.push(u16::from_le_bytes([bytes[index], bytes[index + 1]]));
        index += 2;
    }
    units
}

/// UTF-16 小端字节流转字符串，去掉结尾的 NUL
#[cfg(windows)]
fn decode_wide(bytes: &[u8]) -> String {
    let mut units = wide_units(bytes);
    if let Some(end) = units.iter().position(|unit| *unit == 0) {
        units.truncate(end);
    }
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    const SAMPLE: &str = "\r
HKEY_LOCAL_MACHINE\\SOFTWARE\\WOW6432Node\\GOG.com\\Games\\1207658691\r
    gameName    REG_SZ    Example Quest\r
    path    REG_SZ    D:\\GOG Games\\Example Quest\r
    buildId    REG_SZ    54321\r
\r
HKEY_LOCAL_MACHINE\\SOFTWARE\\WOW6432Node\\GOG.com\\Games\\1207658692\r
    gameName    REG_SZ    Second Game\r
    path    REG_SZ    D:\\GOG Games\\Second\r
";

    #[test]
    #[cfg(windows)]
    fn native_registry_reads_a_well_known_key() {
        // 真机实现此前没有测试盯着，子键语义与测试替身不一致却一直没人发现
        let registry = WindowsRegistry;
        let key = "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";
        let values = registry.values(key);
        assert!(
            values.contains_key("ProductName"),
            "读不到 ProductName：{values:?}"
        );
        assert_ne!(values.get("ProductName").map_or(0, String::len), 0);
        // 子键名必须是名字，不带路径，适配器拿它再拼下一级
        let children = registry.subkeys(key);
        assert!(
            children.iter().all(|name| !name.contains('\\')),
            "子键名不该带路径：{children:?}"
        );
    }

    #[test]
    #[cfg(windows)]
    fn unknown_hive_or_key_is_empty_not_a_panic() {
        let registry = WindowsRegistry;
        assert_eq!(registry.subkeys("NOPE\\SOFTWARE").len(), 0);
        assert!(registry
            .values("HKCU\\SOFTWARE\\gamelift-not-there")
            .is_empty());
    }

    #[test]
    fn map_registry_lists_subkeys_and_values() {
        let registry = MapRegistry::new([
            (
                "HKCU\\SOFTWARE\\Ubisoft\\Launcher\\Installs\\7080".to_owned(),
                ValueMap::from([("InstallDir".to_owned(), "D:\\Games\\Demo".to_owned())]),
            ),
            (
                "HKCU\\SOFTWARE\\Ubisoft\\Launcher\\Installs\\7081".to_owned(),
                ValueMap::new(),
            ),
        ]);
        let mut found = registry.subkeys("HKCU\\SOFTWARE\\Ubisoft\\Launcher\\Installs");
        found.sort();
        assert_eq!(found, vec!["7080".to_owned(), "7081".to_owned()]);
        assert_eq!(
            registry.values("HKCU\\SOFTWARE\\Ubisoft\\Launcher\\Installs\\7080"),
            ValueMap::from([("InstallDir".to_owned(), "D:\\Games\\Demo".to_owned())])
        );
        assert!(registry.values("HKCU\\SOFTWARE\\Nope").is_empty());
    }
}
