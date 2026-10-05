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

/// 真机实现，通过 `reg query` 读取，避免为一次读表引入依赖
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsRegistry;

impl RegistryRead for WindowsRegistry {
    fn subkeys(&self, key: &str) -> Vec<String> {
        query(key)
            .map(|table| table.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn values(&self, key: &str) -> ValueMap {
        query(key)
            .and_then(|mut table| table.remove(key))
            .unwrap_or_default()
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

/// 调用 `reg query` 并把输出解析成键到值的映射
fn query(key: &str) -> Option<BTreeMap<String, ValueMap>> {
    let output = std::process::Command::new("reg")
        .args(["query", key, "/s"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Some(parse(&text))
}

/// 解析 `reg query /s` 的输出
///
/// 段落以 `HKEY_` 开头的行切分，段内每行是「值名 + 类型 + 值」，
/// 三者之间用空白分隔；值本身可能含空格，因此只切前两段
#[must_use]
pub fn parse(text: &str) -> BTreeMap<String, ValueMap> {
    let mut out: BTreeMap<String, ValueMap> = BTreeMap::new();
    let mut current: Option<String> = None;
    for raw in text.lines() {
        let line = raw.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with("HKEY_") {
            current = Some(line.trim().to_owned());
            out.entry(line.trim().to_owned()).or_default();
            continue;
        }
        let Some(key) = current.as_ref() else {
            continue;
        };
        let trimmed = line.trim();
        let mut words = trimmed.split_whitespace();
        let (Some(name), Some(kind)) = (words.next(), words.next()) else {
            continue;
        };
        if !kind.starts_with("REG_") {
            continue;
        }
        // 值从类型之后开始，内部空格属于值本身
        let after_name = trimmed.get(name.len()..).unwrap_or_default().trim_start();
        let value = after_name
            .get(kind.len()..)
            .unwrap_or_default()
            .trim_start()
            .to_owned();
        out.entry(key.clone())
            .or_default()
            .insert(name.to_owned(), value);
    }
    out
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

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
    fn parses_sections_and_values() {
        let table = parse(SAMPLE);
        assert_eq!(table.len(), 2);
        let first = table
            .get("HKEY_LOCAL_MACHINE\\SOFTWARE\\WOW6432Node\\GOG.com\\Games\\1207658691")
            .expect("first key");
        assert_eq!(
            first.get("gameName").map(String::as_str),
            Some("Example Quest")
        );
        assert_eq!(
            first.get("path").map(String::as_str),
            Some("D:\\GOG Games\\Example Quest")
        );
        assert_eq!(first.get("buildId").map(String::as_str), Some("54321"));
    }

    #[test]
    fn keeps_values_with_spaces_intact() {
        let table = parse(SAMPLE);
        let second = table
            .get("HKEY_LOCAL_MACHINE\\SOFTWARE\\WOW6432Node\\GOG.com\\Games\\1207658692")
            .expect("second key");
        assert_eq!(
            second.get("gameName").map(String::as_str),
            Some("Second Game")
        );
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
