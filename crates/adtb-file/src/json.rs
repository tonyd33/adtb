//! NDJSON files: one JSON value per line, read by the JSON mapping.

use adtb_lang::schema::Schema;
use adtb_lang::textual::Textual;
use adtb_lang::types::{Base, Ty};

use super::{
    Found, Shape, Sum, allowed, ctor as of_ctor, key_value_ty, none, parse, record, shape, some,
    text,
};

#[derive(Clone, Debug, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    /// A number as the file spells it.
    Number(String),
    String(String),
    Array,
    /// Members in the file's order.
    Object(Vec<(String, Json)>),
}

impl Json {
    fn member(&self, name: &str) -> Option<&Json> {
        match self {
            Json::Object(ms) => ms.iter().find(|(n, _)| n == name).map(|(_, v)| v),
            _ => None,
        }
    }
}

/// Returns each nonblank line of `text`, with its row read at `row` or why it does not convert.
pub(super) fn records<V: Textual>(
    schema: &Schema,
    text: &str,
    row: &Ty,
) -> Result<Vec<Found<V>>, String> {
    Ok(text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let read = Parser::new(line)
                .document()
                .and_then(|j| read(schema, &j, row));
            (line.to_string(), read)
        })
        .collect())
}

fn read<V: Textual>(schema: &Schema, j: &Json, ty: &Ty) -> Result<V, String> {
    let parse = |t: &str| parse(schema, t, ty);
    match shape(schema, ty) {
        Shape::Scalar => match (j, ty) {
            (
                Json::Number(n),
                Ty::Base(Base::I8 | Base::I16 | Base::I32 | Base::F32 | Base::F64),
            ) => parse(n),
            (Json::Number(_), _) => Err(format!("{ty:?} is a string in its textual syntax")),
            (Json::String(s), _) => parse(s),
            _ => Err(format!("{j:?} is not a value of {ty:?}")),
        },
        Shape::Bool => match j {
            Json::Bool(b) => Ok(of_ctor(u32::from(*b), Vec::new())),
            _ => Err(format!("{j:?} is not a Bool")),
        },
        Shape::Text => match j {
            Json::String(s) => Ok(text(s)),
            _ => Err(format!("{j:?} is not a Text")),
        },
        Shape::Option(inner) => match j {
            Json::String(s) if s == "None" => Ok(none()),
            Json::Object(ms) if ms.len() == 1 && ms[0].0 == "Some" => {
                let value = ms[0]
                    .1
                    .member("value")
                    .ok_or("`Some` has no member `value`")?;
                Ok(some(read(schema, value, &inner)?))
            }
            _ => Err(format!("{j:?} is not an Option")),
        },
        Shape::Names(names, set) => match j {
            Json::String(s) => {
                let i = names
                    .iter()
                    .position(|n| n == s)
                    .ok_or_else(|| format!("`{s}` is not a constructor"))?;
                Ok(of_ctor(allowed(i as u32, s, set.as_ref())?, Vec::new()))
            }
            _ => Err(format!("{j:?} is not a constructor name")),
        },
        Shape::Key(rel) => Ok(V::of_key(read(schema, j, &key_value_ty(schema, rel))?)),
        Shape::Newtype(base) => read(schema, j, &base),
        Shape::Record(fields, declared) => Ok(record(members(schema, j, &fields)?, declared)),
        Shape::Sum(sum) => ctor(schema, j, &sum),
        Shape::Other => Err(format!("{ty:?} has no JSON form")),
    }
}

/// Reads the members of object `j` named by `fields`; a missing member of an `Option` type is
/// `None`.
fn members<V: Textual>(
    schema: &Schema,
    j: &Json,
    fields: &[(String, Ty)],
) -> Result<Vec<(String, V)>, String> {
    if !matches!(j, Json::Object(_)) {
        return Err(format!("{j:?} is not an object"));
    }
    fields
        .iter()
        .map(|(n, t)| {
            let v = match (j.member(n), shape(schema, t)) {
                (Some(m), _) => read(schema, m, t).map_err(|e| format!("member `{n}`: {e}"))?,
                (None, Shape::Option(_)) => none(),
                (None, _) => return Err(format!("no member `{n}`")),
            };
            Ok((n.clone(), v))
        })
        .collect()
}

/// Reads a constructor value: its name for one with no fields, else an object of one member
/// named by it holding its fields.
fn ctor<V: Textual>(schema: &Schema, j: &Json, sum: &Sum) -> Result<V, String> {
    let (name, payload) = match j {
        Json::String(s) => (s.as_str(), None),
        Json::Object(ms) if ms.len() == 1 => (ms[0].0.as_str(), Some(&ms[0].1)),
        _ => return Err(format!("{j:?} is not a constructor value")),
    };
    let c = sum.ctor(name)?;
    let (_, own) = &sum.ctors[c as usize];
    let mut fields: Vec<(String, Ty)> = sum.header.clone();
    fields.extend(own.iter().cloned());
    let values = match payload {
        Some(p) => members(schema, p, &fields).map_err(|e| format!("`{name}`: {e}"))?,
        None if fields.is_empty() => Vec::new(),
        None => return Err(format!("`{name}` has fields")),
    };
    Ok(of_ctor(c, values))
}

struct Parser<'t> {
    text: &'t str,
    at: usize,
}

impl<'t> Parser<'t> {
    fn new(text: &'t str) -> Parser<'t> {
        Parser { text, at: 0 }
    }

    fn fail<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("column {}: {what}", self.at + 1))
    }

    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.at += 1;
        }
    }

    fn eat(&mut self, b: u8) -> bool {
        self.skip_space();
        if self.peek() == Some(b) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn document(mut self) -> Result<Json, String> {
        let v = self.value()?;
        self.skip_space();
        if self.at < self.text.len() {
            return self.fail("text after the value");
        }
        Ok(v)
    }

    fn value(&mut self) -> Result<Json, String> {
        self.skip_space();
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => {
                for (word, v) in [
                    ("true", Json::Bool(true)),
                    ("false", Json::Bool(false)),
                    ("null", Json::Null),
                ] {
                    if self.text[self.at..].starts_with(word) {
                        self.at += word.len();
                        return Ok(v);
                    }
                }
                self.fail("not a JSON value")
            }
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.at += 1;
        let mut out: Vec<(String, Json)> = Vec::new();
        if self.eat(b'}') {
            return Ok(Json::Object(out));
        }
        loop {
            self.skip_space();
            if self.peek() != Some(b'"') {
                return self.fail("a member name");
            }
            let name = self.string()?;
            if !self.eat(b':') {
                return self.fail("`:` after a member name");
            }
            let v = self.value()?;
            if out.iter().any(|(n, _)| *n == name) {
                return self.fail(&format!("member `{name}` twice"));
            }
            out.push((name, v));
            if self.eat(b',') {
                continue;
            }
            if self.eat(b'}') {
                return Ok(Json::Object(out));
            }
            return self.fail("`,` or `}`");
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.at += 1;
        if self.eat(b']') {
            return Ok(Json::Array);
        }
        loop {
            self.value()?;
            if self.eat(b',') {
                continue;
            }
            if self.eat(b']') {
                return Ok(Json::Array);
            }
            return self.fail("`,` or `]`");
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.at;
        while matches!(
            self.peek(),
            Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
        ) {
            self.at += 1;
        }
        Ok(Json::Number(self.text[start..self.at].to_string()))
    }

    fn string(&mut self) -> Result<String, String> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.text[self.at..].chars().next() else {
                return self.fail("an unclosed string");
            };
            self.at += c.len_utf8();
            match c {
                '"' => return Ok(out),
                '\\' => out.push(self.escape()?),
                c if u32::from(c) < 0x20 => return self.fail("a control character in a string"),
                c => out.push(c),
            }
        }
    }

    fn escape(&mut self) -> Result<char, String> {
        let Some(e) = self.peek() else {
            return self.fail("an unfinished escape");
        };
        self.at += 1;
        Ok(match e {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => {
                let hi = self.hex4()?;
                let code = if (0xd800..0xdc00).contains(&hi) {
                    if !self.text[self.at..].starts_with("\\u") {
                        return self.fail("a lone surrogate");
                    }
                    self.at += 2;
                    let lo = self.hex4()?;
                    if !(0xdc00..0xe000).contains(&lo) {
                        return self.fail("a lone surrogate");
                    }
                    0x10000 + ((hi - 0xd800) << 10) + (lo - 0xdc00)
                } else {
                    hi
                };
                match char::from_u32(code) {
                    Some(c) => c,
                    None => return self.fail("a lone surrogate"),
                }
            }
            _ => return self.fail("an unknown escape"),
        })
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self.text.get(self.at..self.at + 4).unwrap_or_default();
        match u32::from_str_radix(digits, 16) {
            Ok(n) if digits.len() == 4 && digits.bytes().all(|b| b.is_ascii_hexdigit()) => {
                self.at += 4;
                Ok(n)
            }
            _ => self.fail("four hex digits after `\\u`"),
        }
    }
}
