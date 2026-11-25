//! Pure-Rust RFC-8259 JSON parser, serializer, and query value tree (zero external crates).

use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    pub fn obj(entries: Vec<(&str, JsonValue)>) -> Self {
        JsonValue::Object(
            entries
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        )
    }

    pub fn arr(items: Vec<JsonValue>) -> Self {
        JsonValue::Array(items)
    }

    pub fn parse(input: &str) -> Result<Self, String> {
        let mut p = Parser {
            bytes: input.as_bytes(),
            pos: 0,
        };
        let v = p.parse_value()?;
        p.skip_ws();
        if p.pos < p.bytes.len() {
            return Err(format!("Unexpected trailing JSON token at byte {}", p.pos));
        }
        Ok(v)
    }

    pub fn to_json_string(&self) -> String {
        let mut out = String::with_capacity(256);
        self.write_into(&mut out);
        out
    }

    fn write_into(&self, out: &mut String) {
        match self {
            JsonValue::Null => out.push_str("null"),
            JsonValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            JsonValue::Number(n) => {
                if n.is_finite() {
                    if n.fract() == 0.0 && n.abs() < (i64::MAX as f64) {
                        out.push_str(&(*n as i64).to_string());
                    } else {
                        out.push_str(&n.to_string());
                    }
                } else {
                    out.push_str("null");
                }
            }
            JsonValue::Str(s) => escape_json_str(s, out),
            JsonValue::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_into(out);
                }
                out.push(']');
            }
            JsonValue::Object(fields) => {
                out.push('{');
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    escape_json_str(k, out);
                    out.push(':');
                    v.write_into(out);
                }
                out.push('}');
            }
        }
    }

    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        if let JsonValue::Object(fields) = self {
            for (k, v) in fields.iter().rev() {
                if k == key {
                    return Some(v);
                }
            }
        }
        None
    }

    pub fn get_index(&self, idx: usize) -> Option<&JsonValue> {
        if let JsonValue::Array(items) = self {
            items.get(idx)
        } else {
            None
        }
    }

    pub fn set(&mut self, key: &str, val: JsonValue) {
        if let JsonValue::Object(fields) = self {
            for (k, v) in fields.iter_mut() {
                if k == key {
                    *v = val;
                    return;
                }
            }
            fields.push((key.to_string(), val));
        }
    }

    pub fn push(&mut self, val: JsonValue) {
        if let JsonValue::Array(items) = self {
            items.push(val);
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        if let JsonValue::Str(s) = self {
            Some(s.as_str())
        } else {
            None
        }
    }

    pub fn str_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.get(key).and_then(|v| v.as_str()).unwrap_or(default)
    }

    pub fn string_or(&self, key: &str, default: &str) -> String {
        self.str_or(key, default).to_string()
    }

    pub fn as_bool(&self) -> Option<bool> {
        if let JsonValue::Bool(b) = self {
            Some(*b)
        } else {
            None
        }
    }

    pub fn bool_or(&self, key: &str, default: bool) -> bool {
        self.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
    }

    pub fn as_i64(&self) -> Option<i64> {
        if let JsonValue::Number(n) = self {
            Some(*n as i64)
        } else {
            None
        }
    }

    pub fn i64_or(&self, key: &str, default: i64) -> i64 {
        self.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
    }

    pub fn as_array(&self) -> Option<&[JsonValue]> {
        if let JsonValue::Array(a) = self {
            Some(a.as_slice())
        } else {
            None
        }
    }
}

impl From<&str> for JsonValue {
    fn from(s: &str) -> Self {
        JsonValue::Str(s.to_string())
    }
}

impl From<String> for JsonValue {
    fn from(s: String) -> Self {
        JsonValue::Str(s)
    }
}

impl From<bool> for JsonValue {
    fn from(b: bool) -> Self {
        JsonValue::Bool(b)
    }
}

impl From<i64> for JsonValue {
    fn from(n: i64) -> Self {
        JsonValue::Number(n as f64)
    }
}

impl From<usize> for JsonValue {
    fn from(n: usize) -> Self {
        JsonValue::Number(n as f64)
    }
}

impl From<f64> for JsonValue {
    fn from(n: f64) -> Self {
        JsonValue::Number(n)
    }
}

impl fmt::Display for JsonValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_json_string())
    }
}

fn escape_json_str(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' | b'\t' | b'\r' | b'\n' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn parse_value(&mut self) -> Result<JsonValue, String> {
        self.skip_ws();
        let b = self
            .peek()
            .ok_or_else(|| "Unexpected end of JSON input".to_string())?;
        match b {
            b'n' => self.expect_ident(b"null", JsonValue::Null),
            b't' => self.expect_ident(b"true", JsonValue::Bool(true)),
            b'f' => self.expect_ident(b"false", JsonValue::Bool(false)),
            b'"' => self.parse_string().map(JsonValue::Str),
            b'[' => self.parse_array(),
            b'{' => self.parse_object(),
            b'-' | b'0'..=b'9' => self.parse_number(),
            other => Err(format!(
                "Unexpected character '{}' at byte {}",
                other as char, self.pos
            )),
        }
    }

    fn expect_ident(&mut self, ident: &[u8], val: JsonValue) -> Result<JsonValue, String> {
        if self.bytes.get(self.pos..self.pos + ident.len()) == Some(ident) {
            self.pos += ident.len();
            Ok(val)
        } else {
            Err(format!("Invalid literal at byte {}", self.pos))
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.pos += 1; // skip '"'
        let mut out = String::new();
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b == b'"' {
                self.pos += 1;
                return Ok(out);
            }
            if b == b'\\' {
                self.pos += 1;
                let esc = *self
                    .bytes
                    .get(self.pos)
                    .ok_or_else(|| "Unterminated escape".to_string())?;
                self.pos += 1;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\x08'),
                    b'f' => out.push('\x0c'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let code = self.parse_hex4()?;
                        if (0xD800..=0xDBFF).contains(&code) {
                            if self.bytes.get(self.pos..self.pos + 2) == Some(b"\\u") {
                                self.pos += 2;
                                let low = self.parse_hex4()?;
                                if (0xDC00..=0xDFFF).contains(&low) {
                                    let combined = 0x10000
                                        + (((code as u32 - 0xD800) << 10) | (low as u32 - 0xDC00));
                                    if let Some(ch) = char::from_u32(combined) {
                                        out.push(ch);
                                        continue;
                                    }
                                }
                            }
                            out.push('\u{FFFD}');
                        } else if let Some(ch) = char::from_u32(code as u32) {
                            out.push(ch);
                        } else {
                            out.push('\u{FFFD}');
                        }
                    }
                    other => out.push(other as char),
                }
            } else {
                // Decode valid UTF-8 sequence starting at self.pos
                let slice = &self.bytes[self.pos..];
                let s = std::str::from_utf8(slice)
                    .ok()
                    .or_else(|| {
                        for len in (1..=4.min(slice.len())).rev() {
                            if let Ok(sub) = std::str::from_utf8(&slice[..len]) {
                                return Some(sub);
                            }
                        }
                        None
                    })
                    .ok_or_else(|| format!("Invalid UTF-8 in JSON string at {}", self.pos))?;
                let ch = s.chars().next().unwrap();
                out.push(ch);
                self.pos += ch.len_utf8();
            }
        }
        Err("Unterminated JSON string".to_string())
    }

    fn parse_hex4(&mut self) -> Result<u16, String> {
        let slice = self
            .bytes
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| "Short \\uXXXX escape".to_string())?;
        let s = std::str::from_utf8(slice).map_err(|_| "Invalid hex digits".to_string())?;
        let val = u16::from_str_radix(s, 16).map_err(|_| "Invalid hex value".to_string())?;
        self.pos += 4;
        Ok(val)
    }

    fn parse_number(&mut self) -> Result<JsonValue, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while let Some(b'0'..=b'9') = self.peek() {
            self.pos += 1;
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            while let Some(b'0'..=b'9') = self.peek() {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            while let Some(b'0'..=b'9') = self.peek() {
                self.pos += 1;
            }
        }
        let s = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| "Invalid number UTF-8".to_string())?;
        let n = s
            .parse::<f64>()
            .map_err(|e| format!("Invalid number '{}': {}", s, e))?;
        Ok(JsonValue::Number(n))
    }

    fn parse_array(&mut self) -> Result<JsonValue, String> {
        self.pos += 1; // '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonValue::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(JsonValue::Array(items));
                }
                _ => return Err(format!("Expected ',' or ']' at byte {}", self.pos)),
            }
        }
    }

    fn parse_object(&mut self) -> Result<JsonValue, String> {
        self.pos += 1; // '{'
        let mut fields = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonValue::Object(fields));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(format!("Expected string key at byte {}", self.pos));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(format!("Expected ':' at byte {}", self.pos));
            }
            self.pos += 1;
            let val = self.parse_value()?;
            fields.push((key, val));
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(JsonValue::Object(fields));
                }
                _ => return Err(format!("Expected ',' or '}}' at byte {}", self.pos)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_complex_json() {
        let v = JsonValue::obj(vec![
            ("provider", "openai".into()),
            ("steps", 15i64.into()),
            ("ok", true.into()),
            (
                "tools",
                JsonValue::arr(vec!["read_file".into(), "edit_file".into()]),
            ),
        ]);
        let s = v.to_json_string();
        let parsed = JsonValue::parse(&s).expect("valid json");
        assert_eq!(parsed.str_or("provider", ""), "openai");
        assert_eq!(parsed.i64_or("steps", 0), 15);
        assert!(parsed.bool_or("ok", false));
        assert_eq!(parsed.get("tools").unwrap().as_array().unwrap().len(), 2);
    }
}
