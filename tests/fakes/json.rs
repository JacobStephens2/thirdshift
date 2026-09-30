//! Just enough JSON for the fakes, with only the standard library: they are
//! built with plain `rustc`, so they can't use serde_json. Objects keep their
//! keys in order, numbers keep their text, and output is laid out as Python's
//! `json` module lays it out: `dump` as `json.dump(…, indent=2)`, `Display`
//! as `json.dumps(…)`, both with non-ASCII escaped.

use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    /// The number as written, so it is written back the same.
    Number(String),
    String(String),
    Array(Vec<Json>),
    /// Keys in insertion order, as a Python dict keeps them.
    Object(Vec<(String, Json)>),
}

pub use Json::{Array, Bool, Null};

/// A JSON string.
pub fn string(text: impl Into<String>) -> Json {
    Json::String(text.into())
}

/// A JSON number.
pub fn number(value: impl fmt::Display) -> Json {
    Json::Number(value.to_string())
}

/// A JSON object with `fields` in order.
pub fn object<const N: usize>(fields: [(&str, Json); N]) -> Json {
    Json::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

impl Json {
    /// The value under `key` of an object; `None` for a missing key or a
    /// value that isn't an object.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Json> {
        match self {
            Json::Object(fields) => fields.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Like `get`, for a key the fake's contract says is always there.
    pub fn at(&self, key: &str) -> &Json {
        self.get(key)
            .unwrap_or_else(|| panic!("no {key:?} in {self}"))
    }

    pub fn at_mut(&mut self, key: &str) -> &mut Json {
        let shown = self.to_string();
        self.get_mut(key)
            .unwrap_or_else(|| panic!("no {key:?} in {shown}"))
    }

    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Set `key` to `value`: in place if the object has it, else at the end,
    /// as assigning to a Python dict does.
    pub fn set(&mut self, key: &str, value: Json) {
        let Json::Object(fields) = self else {
            panic!("setting {key:?} on {self}");
        };
        match fields.iter_mut().find(|(k, _)| k == key) {
            Some((_, slot)) => *slot = value,
            None => fields.push((key.to_owned(), value)),
        }
    }

    /// The value under `key`, first set to `default` if the object has no
    /// such key, as Python's `dict.setdefault` does.
    pub fn entry(&mut self, key: &str, default: Json) -> &mut Json {
        if !self.has(key) {
            self.set(key, default);
        }
        self.at_mut(key)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(text) => Some(text),
            _ => None,
        }
    }

    /// The string, for a value the fake's contract says is one.
    pub fn str(&self) -> &str {
        self.as_str()
            .unwrap_or_else(|| panic!("{self} is not a string"))
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Number(text) => text.parse().ok(),
            _ => None,
        }
    }

    pub fn items(&self) -> &[Json] {
        match self {
            Json::Array(items) => items,
            _ => panic!("{self} is not a list"),
        }
    }

    pub fn items_mut(&mut self) -> &mut Vec<Json> {
        match self {
            Json::Array(items) => items,
            _ => panic!("{self} is not a list"),
        }
    }

    /// False for what Python counts as false: null, false, 0, "", [] and {}.
    pub fn truthy(&self) -> bool {
        match self {
            Json::Null => false,
            Json::Bool(value) => *value,
            Json::Number(text) => text.parse::<f64>().is_ok_and(|n| n != 0.0),
            Json::String(text) => !text.is_empty(),
            Json::Array(items) => !items.is_empty(),
            Json::Object(fields) => !fields.is_empty(),
        }
    }

    /// The value as a Python f-string shows it: strings bare, `None`,
    /// `True` and `False` for the rest of the scalars.
    pub fn python(&self) -> String {
        match self {
            Json::Null => "None".to_owned(),
            Json::Bool(true) => "True".to_owned(),
            Json::Bool(false) => "False".to_owned(),
            Json::String(text) => text.clone(),
            other => other.to_string(),
        }
    }

    /// Laid out as `json.dump(value, f, indent=2)` lays it out.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, Some(0));
        out
    }

    /// `indent` is the current depth when laying out with an indent of 2,
    /// `None` for `json.dumps`'s single line.
    fn write(&self, out: &mut String, indent: Option<usize>) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Json::Number(text) => out.push_str(text),
            Json::String(text) => write_string(out, text),
            Json::Array(items) => {
                write_container(out, indent, '[', ']', items, |out, item, indent| {
                    item.write(out, indent)
                })
            }
            Json::Object(fields) => write_container(
                out,
                indent,
                '{',
                '}',
                fields,
                |out, (key, value), indent| {
                    write_string(out, key);
                    out.push_str(": ");
                    value.write(out, indent);
                },
            ),
        }
    }
}

fn write_container<T>(
    out: &mut String,
    indent: Option<usize>,
    open: char,
    close: char,
    items: &[T],
    write_item: impl Fn(&mut String, &T, Option<usize>),
) {
    out.push(open);
    if items.is_empty() {
        out.push(close);
        return;
    }
    let inner = indent.map(|depth| depth + 1);
    for (at, item) in items.iter().enumerate() {
        match inner {
            Some(depth) => {
                out.push_str(if at == 0 { "\n" } else { ",\n" });
                out.push_str(&"  ".repeat(depth));
            }
            None if at > 0 => out.push_str(", "),
            None => {}
        }
        write_item(out, item, inner);
    }
    if let Some(depth) = indent {
        out.push('\n');
        out.push_str(&"  ".repeat(depth));
    }
    out.push(close);
}

/// `text` quoted as Python's `json` quotes it, with `ensure_ascii`.
fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out.push('"');
}

impl fmt::Display for Json {
    /// On one line, as `json.dumps(value)` writes it.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut out = String::new();
        self.write(&mut out, None);
        f.write_str(&out)
    }
}

/// Parse `text`, which must be one JSON value.
pub fn parse(text: &str) -> Result<Json, String> {
    let mut parser = Parser {
        text: text.as_bytes(),
        at: 0,
    };
    let value = parser.value()?;
    parser.skip_space();
    if parser.at != parser.text.len() {
        return Err(parser.error("trailing text"));
    }
    Ok(value)
}

struct Parser<'a> {
    text: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn error(&self, what: &str) -> String {
        format!("invalid JSON: {what} at byte {}", self.at)
    }

    fn skip_space(&mut self) {
        while self
            .text
            .get(self.at)
            .is_some_and(|b| b.is_ascii_whitespace())
        {
            self.at += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_space();
        self.text.get(self.at).copied()
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.peek() != Some(byte) {
            return Err(self.error(&format!("expected {:?}", byte as char)));
        }
        self.at += 1;
        Ok(())
    }

    fn value(&mut self) -> Result<Json, String> {
        match self.peek() {
            Some(b'{') => {
                self.at += 1;
                let mut fields = Vec::new();
                if self.peek() == Some(b'}') {
                    self.at += 1;
                    return Ok(Json::Object(fields));
                }
                loop {
                    let key = self.string()?;
                    self.expect(b':')?;
                    fields.push((key, self.value()?));
                    match self.peek() {
                        Some(b',') => self.at += 1,
                        Some(b'}') => {
                            self.at += 1;
                            return Ok(Json::Object(fields));
                        }
                        _ => return Err(self.error("expected ',' or '}'")),
                    }
                }
            }
            Some(b'[') => {
                self.at += 1;
                let mut items = Vec::new();
                if self.peek() == Some(b']') {
                    self.at += 1;
                    return Ok(Json::Array(items));
                }
                loop {
                    items.push(self.value()?);
                    match self.peek() {
                        Some(b',') => self.at += 1,
                        Some(b']') => {
                            self.at += 1;
                            return Ok(Json::Array(items));
                        }
                        _ => return Err(self.error("expected ',' or ']'")),
                    }
                }
            }
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b't') => self.word("true", Json::Bool(true)),
            Some(b'f') => self.word("false", Json::Bool(false)),
            Some(b'n') => self.word("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => {
                let start = self.at;
                while self
                    .text
                    .get(self.at)
                    .is_some_and(|b| matches!(b, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'))
                {
                    self.at += 1;
                }
                let text = std::str::from_utf8(&self.text[start..self.at]).unwrap();
                if text.parse::<f64>().is_err() {
                    return Err(self.error("bad number"));
                }
                Ok(Json::Number(text.to_owned()))
            }
            _ => Err(self.error("expected a value")),
        }
    }

    fn word(&mut self, word: &str, value: Json) -> Result<Json, String> {
        if !self.text[self.at..].starts_with(word.as_bytes()) {
            return Err(self.error(&format!("expected {word}")));
        }
        self.at += word.len();
        Ok(value)
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut bytes = Vec::new();
        loop {
            let Some(&byte) = self.text.get(self.at) else {
                return Err(self.error("unterminated string"));
            };
            self.at += 1;
            match byte {
                b'"' => break,
                b'\\' => {
                    let Some(&escape) = self.text.get(self.at) else {
                        return Err(self.error("unterminated string"));
                    };
                    self.at += 1;
                    let c = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.unicode_escape()?,
                        _ => return Err(self.error("bad escape")),
                    };
                    bytes.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                }
                _ => bytes.push(byte),
            }
        }
        String::from_utf8(bytes).map_err(|_| self.error("invalid UTF-8"))
    }

    /// The character of a `\uXXXX` escape, after its `\u`, taking the low
    /// half of a surrogate pair too.
    fn unicode_escape(&mut self) -> Result<char, String> {
        let high = self.hex4()?;
        if !(0xD800..0xDC00).contains(&high) {
            return char::from_u32(high).ok_or_else(|| self.error("bad \\u escape"));
        }
        if !self.text[self.at..].starts_with(b"\\u") {
            return Err(self.error("lone surrogate"));
        }
        self.at += 2;
        let low = self.hex4()?;
        char::from_u32(0x10000 + ((high - 0xD800) << 10) + (low.wrapping_sub(0xDC00) & 0x3FF))
            .ok_or_else(|| self.error("bad surrogate pair"))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .text
            .get(self.at..self.at + 4)
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| u32::from_str_radix(digits, 16).ok())
            .ok_or_else(|| self.error("bad \\u escape"))?;
        self.at += 4;
        Ok(digits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATE: &str = r#"{"repo": "acme/widgets", "prs": [], "checks": {"abc": [{"name": "build", "pending_polls": 2, "url": null, "ok": true}]}, "note": "caf\u00e9 \ud83d\ude00 \"q\"\n"}"#;

    #[test]
    fn prints_json_on_one_line_as_python_json_dumps_does() {
        let state = parse(STATE).unwrap();
        assert_eq!(state.to_string(), STATE);
    }

    #[test]
    fn dumps_json_with_an_indent_of_two_as_python_json_dump_does() {
        let state = parse(STATE).unwrap();
        assert_eq!(
            state.dump(),
            r#"{
  "repo": "acme/widgets",
  "prs": [],
  "checks": {
    "abc": [
      {
        "name": "build",
        "pending_polls": 2,
        "url": null,
        "ok": true
      }
    ]
  },
  "note": "caf\u00e9 \ud83d\ude00 \"q\"\n"
}"#
        );
    }

    #[test]
    fn reads_escapes_back_as_the_characters_they_stand_for() {
        let state = parse(STATE).unwrap();
        assert_eq!(state.at("note").str(), "café 😀 \"q\"\n");
        assert_eq!(parse(r#""a\/b\tc""#).unwrap(), string("a/b\tc"));
    }

    #[test]
    fn setting_a_key_keeps_its_place_and_a_new_key_goes_last() {
        let mut state = parse(STATE).unwrap();
        state.set("repo", string("acme/gadgets"));
        state.set("user_email", Json::Null);
        let renamed = STATE.replace("widgets", "gadgets");
        let expected = format!(
            "{}, \"user_email\": null}}",
            renamed.strip_suffix('}').unwrap()
        );
        assert_eq!(state.to_string(), expected);
    }

    #[test]
    fn rejects_text_that_is_not_one_json_value() {
        assert!(parse("{\"a\": 1} x").is_err());
        assert!(parse("[1, ]").is_err());
        assert!(parse("\"open").is_err());
    }
}
