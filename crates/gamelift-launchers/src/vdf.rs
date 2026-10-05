//! 极简 VDF 文本格式解析器，覆盖 Steam `appmanifest_*.acf` 与
//! `libraryfolders.vdf` 所需的子集：带引号的键值、嵌套对象、`//` 注释
//! Steam 的 acf 键值恒为带引号字符串，故不处理裸 token

use std::fmt;

/// VDF 值：字符串或有序键值对对象，acf 中 depot 顺序有意义故用 Vec 而非 Map
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VdfValue {
    /// 带引号的字符串值
    Str(String),
    /// 对象：按出现顺序保存的键值对
    Obj(Vec<(String, VdfValue)>),
}

impl VdfValue {
    /// 取对象中第一个匹配键的值
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&VdfValue> {
        let VdfValue::Obj(pairs) = self else {
            return None;
        };
        pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// 作为字符串值取出
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            VdfValue::Str(s) => Some(s),
            VdfValue::Obj(_) => None,
        }
    }
}

/// 解析失败原因
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    /// 文本在结构中间结束
    #[error("VDF 意外结束")]
    UnexpectedEof,
    /// 出现不符合语法的 token
    #[error("VDF 意外 token: {0}")]
    UnexpectedToken(String),
    /// 引号未闭合，或字符串内容不是合法 UTF-8
    #[error("VDF 字符串未闭合或非 UTF-8（起始位置 {0}）")]
    UnterminatedString(usize),
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

/// 解析一段 VDF 文本，返回顶层对象
///
/// # Errors
///
/// 文本不符合 VDF 语法时返回 [`ParseError`]
pub fn parse(text: &str) -> Result<VdfValue, ParseError> {
    let mut p = Parser {
        bytes: text.as_bytes(),
        pos: 0,
    };
    p.parse_object()
}

impl Parser<'_> {
    fn skip_ws_and_comments(&mut self) {
        loop {
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            if self.bytes.get(self.pos..self.pos + 2) == Some(b"//") {
                while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    /// 读取一个带引号的字符串，假定当前 pos 指向 `"`
    /// 非转义内容按 UTF-8 透传，acf 可能出现中文游戏目录名
    fn read_quoted(&mut self) -> Result<String, ParseError> {
        let start = self.pos;
        self.pos += 1; // 跳过开头引号
        let mut out = Vec::new();
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'"' => {
                    self.pos += 1;
                    return String::from_utf8(out)
                        .map_err(|_| ParseError::UnterminatedString(start));
                }
                b'\\' if matches!(self.bytes.get(self.pos + 1), Some(b'"' | b'\\')) => {
                    out.push(self.bytes[self.pos + 1]);
                    self.pos += 2;
                }
                ch => {
                    out.push(ch);
                    self.pos += 1;
                }
            }
        }
        Err(ParseError::UnterminatedString(start))
    }

    /// 下一个 token：`{`、`}` 或带引号字符串
    fn next_token(&mut self) -> Result<Option<Token>, ParseError> {
        self.skip_ws_and_comments();
        if self.pos >= self.bytes.len() {
            return Ok(None);
        }
        match self.bytes[self.pos] {
            b'{' => {
                self.pos += 1;
                Ok(Some(Token::OpenBrace))
            }
            b'}' => {
                self.pos += 1;
                Ok(Some(Token::CloseBrace))
            }
            b'"' => Ok(Some(Token::Str(self.read_quoted()?))),
            other => Err(ParseError::UnexpectedToken(
                String::from_utf8_lossy(&[other]).into_owned(),
            )),
        }
    }

    fn parse_object(&mut self) -> Result<VdfValue, ParseError> {
        let mut pairs = Vec::new();
        // 顶层直接是 `"AppState" { ... }` 时按隐式对象处理并递归
        while let Some(token) = self.next_token()? {
            match token {
                Token::CloseBrace => break,
                Token::OpenBrace => return Err(ParseError::UnexpectedToken("{".into())),
                Token::Str(key) => {
                    let value = self.parse_value()?;
                    pairs.push((key, value));
                }
            }
        }
        Ok(VdfValue::Obj(pairs))
    }

    /// 已读到 key，解析其值：`{` 开对象，否则必须是字符串
    fn parse_value(&mut self) -> Result<VdfValue, ParseError> {
        let Some(token) = self.next_token()? else {
            return Err(ParseError::UnexpectedEof);
        };
        match token {
            Token::OpenBrace => self.parse_object(),
            Token::CloseBrace => Err(ParseError::UnexpectedToken("}".into())),
            Token::Str(s) => Ok(VdfValue::Str(s)),
        }
    }
}

enum Token {
    OpenBrace,
    CloseBrace,
    Str(String),
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::OpenBrace => write!(f, "{{"),
            Token::CloseBrace => write!(f, "}}"),
            Token::Str(s) => write!(f, "\"{s}\""),
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    /// 合成样本：acf 的关键字段结构
    const ACF_SAMPLE: &str = r#"
"AppState"
{
	"appid"		"123456"
	"StateFlags"		"4"
	"installdir"		"example_game"
	"SizeOnDisk"		"123456789"
	"buildid"		"1000000"
	"InstalledDepots"
	{
		"123457"
		{
			"manifest"		"1000000000000000000"
			"size"		"123456789"
		}
	}
}
"#;

    #[test]
    fn parses_acf_sample() {
        let v = parse(ACF_SAMPLE).expect("acf must parse");
        let app = v.get("AppState").expect("AppState key");
        assert_eq!(app.get("appid").and_then(VdfValue::as_str), Some("123456"));
        assert_eq!(
            app.get("buildid").and_then(VdfValue::as_str),
            Some("1000000")
        );
        assert_eq!(
            app.get("installdir").and_then(VdfValue::as_str),
            Some("example_game")
        );
        let depots = app.get("InstalledDepots").expect("depots");
        let depot = depots.get("123457").expect("depot 123457");
        assert_eq!(
            depot.get("manifest").and_then(VdfValue::as_str),
            Some("1000000000000000000")
        );
    }

    #[test]
    fn parses_libraryfolders_vdf() {
        let text = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"D:\\Program Files (x86)\\Steam"
	}
	"1"
	{
		"path"		"E:\\SteamLibrary"
	}
}
"#;
        let v = parse(text).expect("vdf must parse");
        let libs = v.get("libraryfolders").expect("libraryfolders");
        assert_eq!(
            libs.get("0")
                .and_then(|x| x.get("path"))
                .and_then(VdfValue::as_str),
            Some("D:\\Program Files (x86)\\Steam")
        );
        assert_eq!(
            libs.get("1")
                .and_then(|x| x.get("path"))
                .and_then(VdfValue::as_str),
            Some("E:\\SteamLibrary")
        );
    }

    #[test]
    fn handles_comments_and_escapes() {
        let text = "// 一行注释\n\"key\" \"va\\\"lue\" // 行尾注释\n";
        let v = parse(text).expect("must parse");
        assert_eq!(v.get("key").and_then(VdfValue::as_str), Some("va\"lue"));
    }

    #[test]
    fn unterminated_string_is_error() {
        let err = parse("\"key\" \"oops").expect_err("must fail");
        assert_eq!(err, ParseError::UnterminatedString(6));
    }

    #[test]
    fn missing_value_is_eof_error() {
        let err = parse("\"key\"").expect_err("must fail");
        assert_eq!(err, ParseError::UnexpectedEof);
    }

    #[test]
    fn stray_close_brace_at_top_is_tolerated() {
        // 顶层多余的 `}` 按对象结束处理，Steam 自身产出较宽松
        let v = parse("\"a\" \"1\"\n}").expect("must parse");
        assert_eq!(v.get("a").and_then(VdfValue::as_str), Some("1"));
    }

    #[test]
    fn multibyte_utf8_strings_roundtrip() {
        // 回归：中文游戏目录名曾因单字节透传变乱码
        let v = parse("\"installdir\" \"一起扫雷\"").expect("must parse");
        assert_eq!(
            v.get("installdir").and_then(VdfValue::as_str),
            Some("一起扫雷")
        );
    }
}
