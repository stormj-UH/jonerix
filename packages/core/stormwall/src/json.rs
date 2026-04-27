//! Minimal JSON reader — no external dependencies.
//!
//! Covers the shape stormwall (and upstream nft) emit / consume:
//! objects, arrays, strings, numbers, booleans, null. Enough to drive
//! `nft -j -f <file>`, which is the only JSON-input codepath the test
//! framework requires for its `--check` sanity pass.

use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

impl Value {
    pub fn as_object(&self) -> Option<&BTreeMap<String, Value>> {
        if let Value::Object(m) = self { Some(m) } else { None }
    }
    pub fn as_array(&self) -> Option<&Vec<Value>> {
        if let Value::Array(a) = self { Some(a) } else { None }
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Value::String(s) = self { Some(s) } else { None }
    }
    pub fn as_u64(&self) -> Option<u64> {
        if let Value::Number(n) = self {
            if *n >= 0.0 && n.fract() == 0.0 { Some(*n as u64) } else { None }
        } else { None }
    }
    pub fn as_i64(&self) -> Option<i64> {
        if let Value::Number(n) = self {
            if n.fract() == 0.0 { Some(*n as i64) } else { None }
        } else { None }
    }
}

pub fn parse(src: &str) -> Result<Value, String> {
    let mut p = Parser { bytes: src.as_bytes(), pos: 0 };
    p.skip_ws();
    let v = p.parse_value()?;
    p.skip_ws();
    if p.pos != p.bytes.len() {
        return Err(format!("trailing garbage at byte {}", p.pos));
    }
    Ok(v)
}

struct Parser<'a> { bytes: &'a [u8], pos: usize }

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> { self.bytes.get(self.pos).copied() }
    fn eat(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }
    fn expect(&mut self, b: u8) -> Result<(), String> {
        match self.eat() {
            Some(c) if c == b => Ok(()),
            Some(c) => Err(format!("expected '{}' at byte {}, got '{}'", b as char, self.pos - 1, c as char)),
            None => Err(format!("expected '{}' at end of input", b as char)),
        }
    }
    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            match b { b' ' | b'\t' | b'\n' | b'\r' => { self.pos += 1; } _ => break }
        }
    }

    fn parse_value(&mut self) -> Result<Value, String> {
        self.skip_ws();
        let b = self.peek().ok_or("unexpected end")?;
        match b {
            b'{' => self.parse_object(),
            b'[' => self.parse_array(),
            b'"' => Ok(Value::String(self.parse_string()?)),
            b't' | b'f' => self.parse_bool(),
            b'n' => self.parse_null(),
            c if c == b'-' || (c.is_ascii_digit()) => self.parse_number(),
            other => Err(format!("unexpected '{}' at byte {}", other as char, self.pos)),
        }
    }

    fn parse_object(&mut self) -> Result<Value, String> {
        self.expect(b'{')?;
        let mut map = BTreeMap::new();
        self.skip_ws();
        if self.peek() == Some(b'}') { self.pos += 1; return Ok(Value::Object(map)); }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(b':')?;
            let v = self.parse_value()?;
            map.insert(key, v);
            self.skip_ws();
            match self.peek() {
                Some(b',') => { self.pos += 1; continue; }
                Some(b'}') => { self.pos += 1; break; }
                Some(c) => return Err(format!("expected ',' or '}}' at byte {}, got '{}'", self.pos, c as char)),
                None => return Err("unterminated object".into()),
            }
        }
        Ok(Value::Object(map))
    }

    fn parse_array(&mut self) -> Result<Value, String> {
        self.expect(b'[')?;
        let mut arr = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') { self.pos += 1; return Ok(Value::Array(arr)); }
        loop {
            let v = self.parse_value()?;
            arr.push(v);
            self.skip_ws();
            match self.peek() {
                Some(b',') => { self.pos += 1; continue; }
                Some(b']') => { self.pos += 1; break; }
                Some(c) => return Err(format!("expected ',' or ']' at byte {}, got '{}'", self.pos, c as char)),
                None => return Err("unterminated array".into()),
            }
        }
        Ok(Value::Array(arr))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            match self.eat() {
                Some(b'"') => return Ok(out),
                Some(b'\\') => match self.eat() {
                    Some(b'"') => out.push('"'),
                    Some(b'\\') => out.push('\\'),
                    Some(b'/') => out.push('/'),
                    Some(b'n') => out.push('\n'),
                    Some(b'r') => out.push('\r'),
                    Some(b't') => out.push('\t'),
                    Some(b'b') => out.push('\u{0008}'),
                    Some(b'f') => out.push('\u{000c}'),
                    Some(b'u') => {
                        let mut h = String::new();
                        for _ in 0..4 { h.push(self.eat().ok_or("bad \\u")? as char); }
                        let cp = u32::from_str_radix(&h, 16).map_err(|_| "bad \\u hex")?;
                        if let Some(c) = char::from_u32(cp) { out.push(c); }
                    }
                    Some(c) => return Err(format!("bad escape \\{}", c as char)),
                    None => return Err("unterminated escape".into()),
                },
                Some(b) => out.push(b as char), // NOTE: treats UTF-8 byte-wise; fine for ascii JSON
                None => return Err("unterminated string".into()),
            }
        }
    }

    fn parse_number(&mut self) -> Result<Value, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') { self.pos += 1; }
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() || b == b'.' || b == b'e' || b == b'E' || b == b'+' || b == b'-' {
                self.pos += 1;
            } else { break }
        }
        let s = std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|e| e.to_string())?;
        s.parse::<f64>().map(Value::Number).map_err(|e| e.to_string())
    }

    fn parse_bool(&mut self) -> Result<Value, String> {
        if self.bytes[self.pos..].starts_with(b"true") { self.pos += 4; Ok(Value::Bool(true)) }
        else if self.bytes[self.pos..].starts_with(b"false") { self.pos += 5; Ok(Value::Bool(false)) }
        else { Err(format!("bad bool at byte {}", self.pos)) }
    }

    fn parse_null(&mut self) -> Result<Value, String> {
        if self.bytes[self.pos..].starts_with(b"null") { self.pos += 4; Ok(Value::Null) }
        else { Err(format!("bad null at byte {}", self.pos)) }
    }
}
