use std::collections::BTreeSet;
use std::fmt;

use adtb_lang::lex;
use adtb_lang::schema::{Schema, TypeDefKind};
use adtb_lang::types::{Base, RelId, Ty};

use super::print::{is_prelude, key_field_ty};
use crate::num::in_range;
use crate::time::{MICROS_PER_DAY, at_day, date_in_range, days_from_civil, days_in_month};
use crate::value::{Value, canon32, canon64};
use crate::zone;

/// Why text does not read as a value: the byte offset where reading stopped, and what it
/// expected there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub offset: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at byte {}: {}", self.offset, self.message)
    }
}

impl std::error::Error for ParseError {}

type P<T> = Result<T, ParseError>;

/// Reads `text`, the textual syntax of one value, at type `ty`.
pub fn parse(schema: &Schema, text: &str, ty: &Ty) -> P<Value> {
    let mut r = Reader::new(schema, text);
    let v = r.value(ty)?;
    r.end()?;
    Ok(v)
}

pub(super) struct Reader<'s> {
    schema: &'s Schema,
    pub(super) src: &'s str,
    pub(super) pos: usize,
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic()
}

fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

impl<'s> Reader<'s> {
    pub(super) fn new(schema: &'s Schema, src: &'s str) -> Reader<'s> {
        Reader {
            schema,
            src,
            pos: 0,
        }
    }

    pub(super) fn fail<T>(&self, message: impl Into<String>) -> P<T> {
        Err(ParseError {
            offset: self.pos,
            message: message.into(),
        })
    }

    fn rest(&self) -> &'s str {
        &self.src[self.pos..]
    }

    fn peek(&self) -> Option<u8> {
        self.src.as_bytes().get(self.pos).copied()
    }

    pub(super) fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.pos += 1;
        }
    }

    pub(super) fn at_end(&mut self) -> bool {
        self.ws();
        self.pos == self.src.len()
    }

    pub(super) fn end(&mut self) -> P<()> {
        if self.at_end() {
            Ok(())
        } else {
            self.fail("expected the end of the text")
        }
    }

    /// Consumes `s` exactly, with no whitespace before it.
    fn glued(&mut self, s: &str) -> bool {
        if self.rest().starts_with(s) {
            self.pos += s.len();
            true
        } else {
            false
        }
    }

    fn eat(&mut self, s: &str) -> bool {
        self.ws();
        self.glued(s)
    }

    pub(super) fn expect(&mut self, s: &str) -> P<()> {
        if self.eat(s) {
            Ok(())
        } else {
            self.fail(format!("expected `{s}`"))
        }
    }

    pub(super) fn ident(&mut self) -> P<&'s str> {
        self.ws();
        let start = self.pos;
        if !self.peek().is_some_and(is_ident_start) {
            return self.fail("expected a name");
        }
        while self.peek().is_some_and(is_ident_char) {
            self.pos += 1;
        }
        Ok(&self.src[start..self.pos])
    }

    /// Reads a name, qualified by a module or not, and returns its last part.
    pub(super) fn name(&mut self) -> P<&'s str> {
        let first = self.ident()?;
        if self.glued("::") {
            return self.ident();
        }
        Ok(first)
    }

    fn digits(&mut self) -> &'s str {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        &self.src[start..self.pos]
    }

    pub(super) fn some_digits(&mut self) -> P<&'s str> {
        let d = self.digits();
        if d.is_empty() {
            return self.fail("expected a digit");
        }
        Ok(d)
    }

    fn fixed_digits(&mut self, n: usize) -> P<u32> {
        let start = self.pos;
        let d = self.digits();
        if d.len() != n {
            self.pos = start;
            return self.fail(format!("expected {n} digits"));
        }
        Ok(d.parse().expect("digits"))
    }

    fn sign(&mut self) -> bool {
        self.ws();
        if self.glued("-") {
            self.ws();
            true
        } else {
            false
        }
    }

    fn integer(&mut self, b: Base) -> P<Value> {
        let neg = self.sign();
        let d = self.some_digits()?.trim_start_matches('0');
        let v = d.parse::<i128>().unwrap_or(0);
        let v = if neg { -v } else { v };
        match (d.len() <= 20).then(|| in_range(v, b)).flatten() {
            Some(x) => Ok(Value::Int(x)),
            None => self.fail("an integer out of range of its type"),
        }
    }

    fn decimal(&mut self, s: u8) -> P<Value> {
        let neg = self.sign();
        let whole = self.some_digits()?.trim_start_matches('0');
        let frac = if self.glued(".") {
            self.some_digits()?
        } else {
            ""
        };
        if frac.len() > s as usize {
            return self.fail(format!("more than {s} fractional digits"));
        }
        let digits = format!("{whole}{frac:0<width$}", width = s as usize);
        let digits = digits.trim_start_matches('0');
        if digits.len() > 38 {
            return self.fail("a decimal of more than 38 digits");
        }
        let v = digits.parse::<i128>().unwrap_or(0);
        Ok(Value::Decimal(if neg { -v } else { v }, s))
    }

    fn float(&mut self) -> P<f64> {
        self.ws();
        if self.glued("NaN") {
            return Ok(f64::NAN);
        }
        if self.glued("inf") {
            return Ok(f64::INFINITY);
        }
        if self.glued("-inf") {
            return Ok(f64::NEG_INFINITY);
        }
        let start = self.pos;
        let neg = self.sign();
        let num_start = self.pos;
        self.some_digits()?;
        if self.peek() == Some(b'.') {
            self.pos += 1;
            self.some_digits()?;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            self.some_digits()?;
        }
        let f: f64 = self.src[num_start..self.pos]
            .parse()
            .map_err(|_| ParseError {
                offset: start,
                message: "expected a number".into(),
            })?;
        if f.is_infinite() {
            self.pos = start;
            return self.fail("a number out of range of its type");
        }
        Ok(if neg { -f } else { f })
    }

    fn float32(&mut self) -> P<f32> {
        self.ws();
        let start = self.pos;
        let f = self.float()?;
        if !f.is_finite() {
            return Ok(f as f32);
        }
        let text: String = self.src[start..self.pos]
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        match text.parse::<f32>() {
            Ok(g) if g.is_finite() => Ok(g),
            _ => {
                self.pos = start;
                self.fail("a number out of range of its type")
            }
        }
    }

    fn text(&mut self) -> P<String> {
        self.ws();
        let start = self.pos;
        if !self.glued("\"") {
            return self.fail("expected a text literal");
        }
        loop {
            match self.peek() {
                None | Some(b'\n') => return self.fail("an unterminated text literal"),
                Some(b'"') => {
                    self.pos += 1;
                    break;
                }
                Some(b'\\') => self.pos += 2.min(self.src.len() - self.pos),
                Some(_) => {
                    self.pos += self.rest().chars().next().map_or(1, char::len_utf8);
                }
            }
        }
        let raw = &self.src[start..self.pos];
        let (_, diags) = lex::lex(raw);
        if let Some(d) = diags.first() {
            return Err(ParseError {
                offset: start + d.span.start as usize,
                message: d.message.clone(),
            });
        }
        Ok(lex::decode_text(raw))
    }

    fn hex(&mut self) -> P<Vec<u8>> {
        self.ws();
        if !self.glued("0x") {
            return self.fail("expected `0x`");
        }
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
            self.pos += 1;
        }
        let digits = &self.src[start..self.pos];
        if digits.len() % 2 == 1 {
            return self.fail("an odd number of hex digits");
        }
        Ok((0..digits.len() / 2)
            .map(|i| u8::from_str_radix(&digits[2 * i..2 * i + 2], 16).expect("hex digits"))
            .collect())
    }

    fn date(&mut self) -> P<i64> {
        self.ws();
        let year: i64 = match self.peek() {
            Some(c @ (b'+' | b'-')) => {
                self.pos += 1;
                let d = self.some_digits()?;
                if d.len() < 5 {
                    return self.fail("an expanded year has at least five digits");
                }
                let y: i64 = match d.parse() {
                    Ok(y) => y,
                    Err(_) => return self.fail("a year out of range"),
                };
                if c == b'-' { -y } else { y }
            }
            _ => self.fixed_digits(4)? as i64,
        };
        if !self.glued("-") {
            return self.fail("expected `-`");
        }
        let month = self.fixed_digits(2)?;
        if !self.glued("-") {
            return self.fail("expected `-`");
        }
        let day = self.fixed_digits(2)?;
        if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
            return self.fail("no such date");
        }
        if year.unsigned_abs() > 300_000 {
            return self.fail("a date out of range");
        }
        let days = days_from_civil(year, month, day);
        if !date_in_range(days) {
            return self.fail("a date out of range");
        }
        Ok(days)
    }

    fn time(&mut self) -> P<i64> {
        self.ws();
        let h = self.fixed_digits(2)?;
        if !self.glued(":") {
            return self.fail("expected `:`");
        }
        let m = self.fixed_digits(2)?;
        if !self.glued(":") {
            return self.fail("expected `:`");
        }
        let s = self.fixed_digits(2)?;
        if h > 23 || m > 59 || s > 59 {
            return self.fail("no such time of day");
        }
        let mut micros = 0;
        if self.glued(".") {
            let f = self.some_digits()?;
            if f.len() > 6 {
                return self.fail("more than six fractional digits of a second");
            }
            micros = format!("{f:0<6}").parse::<i64>().expect("digits");
        }
        Ok(((h as i64 * 60 + m as i64) * 60 + s as i64) * 1_000_000 + micros)
    }

    fn datetime(&mut self) -> P<i64> {
        let days = self.date()?;
        if !self.glued("T") {
            return self.fail("expected `T`");
        }
        let t = self.time()?;
        match at_day(days, t) {
            Some(v) => Ok(v),
            None => self.fail("a date and time out of range"),
        }
    }

    fn duration(&mut self) -> P<i64> {
        let neg = self.sign();
        let d = self.some_digits()?.trim_start_matches('0');
        let unit: i64 = if self.glued("us") {
            1
        } else if self.glued("ms") {
            1_000
        } else if self.glued("min") {
            60_000_000
        } else if self.glued("s") {
            1_000_000
        } else if self.glued("h") {
            3_600_000_000
        } else if self.glued("d") {
            MICROS_PER_DAY
        } else {
            return self.fail("expected a unit of time");
        };
        let n = d.parse::<i128>().unwrap_or(0);
        let v = if neg { -n } else { n } * unit as i128;
        match (d.len() <= 20).then(|| i64::try_from(v).ok()).flatten() {
            Some(v) => Ok(v),
            None => self.fail("a duration out of range"),
        }
    }

    fn count32(&mut self) -> P<i32> {
        let neg = self.glued("-");
        let d = self.some_digits()?.trim_start_matches('0');
        let n = d.parse::<i64>().unwrap_or(0);
        match (d.len() <= 11).then(|| i32::try_from(if neg { -n } else { n }).ok()) {
            Some(Some(v)) => Ok(v),
            _ => self.fail("a count out of range of `I32`"),
        }
    }

    fn period(&mut self) -> P<Value> {
        self.ws();
        if !self.glued("P") {
            return self.fail("expected `P`");
        }
        let months = self.count32()?;
        if !self.glued("M") {
            return self.fail("expected `M`");
        }
        let days = self.count32()?;
        if !self.glued("D") {
            return self.fail("expected `D`");
        }
        Ok(Value::Period { months, days })
    }

    fn zone(&mut self) -> P<String> {
        if !self.glued("[") {
            return self.fail("expected `[`");
        }
        let start = self.pos;
        let Some(len) = self.rest().find(']') else {
            return self.fail("expected `]`");
        };
        let name = &self.src[start..start + len];
        if !zone::exists(name) {
            return self.fail(format!("no zone `{name}` in the zone rules"));
        }
        self.pos += len + 1;
        Ok(name.to_string())
    }

    fn base(&mut self, b: Base) -> P<Value> {
        Ok(match b {
            Base::I8 | Base::I16 | Base::I32 | Base::I64 => self.integer(b)?,
            Base::Decimal(s) => self.decimal(s)?,
            Base::F64 => Value::F64(canon64(self.float()?)),
            Base::F32 => Value::F32(canon32(self.float32()?)),
            Base::Text => Value::Text(self.text()?),
            Base::Bytes => Value::Bytes(self.hex()?),
            Base::SortKey => Value::SortKey(self.hex()?),
            Base::Instant => {
                let t = self.datetime()?;
                if !self.glued("Z") {
                    return self.fail("expected `Z`");
                }
                Value::Instant(t)
            }
            Base::Date => Value::Date(self.date()?),
            Base::Time => Value::Time(self.time()?),
            Base::DateTime => Value::DateTime(self.datetime()?),
            Base::ZonedDateTime => {
                let t = self.datetime()?;
                Value::Zoned(t, self.zone()?)
            }
            Base::Duration => Value::Duration(self.duration()?),
            Base::Period => self.period()?,
        })
    }

    fn fields(&mut self, tys: &[(String, Ty)]) -> P<Vec<(String, Value)>> {
        self.expect("{")?;
        self.field_list(tys, "}")
    }

    /// Reads `f: v, ...` up to and including `close`, with every field of `tys` once, in any
    /// order, and returns the values in the order of `tys`.
    fn field_list(&mut self, tys: &[(String, Ty)], close: &str) -> P<Vec<(String, Value)>> {
        let mut got: Vec<Option<Value>> = vec![None; tys.len()];
        if !self.eat(close) {
            loop {
                let at = self.pos;
                let name = self.ident()?;
                let Some(i) = tys.iter().position(|(n, _)| n == name) else {
                    self.pos = at;
                    return self.fail(format!("no field `{name}`"));
                };
                if got[i].is_some() {
                    self.pos = at;
                    return self.fail(format!("field `{name}` given twice"));
                }
                self.expect(":")?;
                got[i] = Some(self.value(&tys[i].1)?);
                if self.eat(close) {
                    break;
                }
                self.expect(",")?;
            }
        }
        tys.iter()
            .zip(got)
            .map(|((n, _), v)| match v {
                Some(v) => Ok((n.clone(), v)),
                None => self.fail(format!("field `{n}` is missing")),
            })
            .collect()
    }

    fn ctor_name(&mut self) -> P<&'s str> {
        self.ws();
        if self.glued("true") {
            return Ok("True");
        }
        if self.glued("false") {
            return Ok("False");
        }
        self.name()
    }

    fn key(&mut self, rel: RelId) -> P<Value> {
        let schema = self.schema;
        let r = schema.relation(rel);
        let at = self.pos;
        if self.name()? != r.name {
            self.pos = at;
            return self.fail(format!("expected a key of `{}`", r.name));
        }
        if !self.glued(".key(") {
            return self.fail("expected `.key(`");
        }
        let v = match r.key.as_slice() {
            [f] => {
                let v = self.value(&key_field_ty(schema, rel, f))?;
                self.expect(")")?;
                v
            }
            fs => {
                let mut tys: Vec<(String, Ty)> = fs
                    .iter()
                    .map(|f| (f.clone(), key_field_ty(schema, rel, f)))
                    .collect();
                tys.sort_by(|a, b| a.0.cmp(&b.0));
                Value::Record(self.field_list(&tys, ")")?)
            }
        };
        Ok(Value::key(v))
    }

    fn data(&mut self, ty: &Ty) -> P<Value> {
        let schema = self.schema;
        let Ty::Data { def, args, ctors } = ty else {
            unreachable!()
        };
        let d = schema.ty(*def);
        match &d.kind {
            TypeDefKind::Newtype { .. } if is_prelude(schema, ty, "Uuid") => {
                self.expect("uuid")?;
                if !self.glued("(") {
                    return self.fail("expected `(`");
                }
                let at = self.pos;
                let t = self.text()?;
                let Some(b) = crate::prim::parse_uuid(&t) else {
                    self.pos = at;
                    return self.fail("not a UUID in the 8-4-4-4-12 form");
                };
                self.expect(")")?;
                Ok(Value::Bytes(b))
            }
            TypeDefKind::Newtype { base, .. } => {
                let at = self.pos;
                if self.name()? != d.name {
                    self.pos = at;
                    return self.fail(format!("expected `{}(`", d.name));
                }
                if !self.glued("(") {
                    return self.fail("expected `(`");
                }
                let v = self.value(base)?;
                self.expect(")")?;
                Ok(v)
            }
            TypeDefKind::Sum { .. } if is_prelude(schema, ty, "Span") => {
                let at = self.pos;
                self.expect("span")?;
                if !self.glued("(") {
                    return self.fail("expected `(`");
                }
                let start = self.value(&args[0])?;
                self.expect(",")?;
                let end = self.value(&args[0])?;
                self.expect(")")?;
                if start > end {
                    self.pos = at;
                    return self.fail("a span whose start is after its end");
                }
                Ok(Value::Ctor(
                    0,
                    vec![("start".into(), start), ("end".into(), end)],
                ))
            }
            TypeDefKind::Sum {
                header, ctors: all, ..
            } => {
                let at = self.pos;
                let name = self.ctor_name()?;
                let c = match d.ctor_index(name) {
                    Some(c) if ctors.as_ref().is_none_or(|s| s.contains(&c)) => c,
                    _ => {
                        self.pos = at;
                        return self.fail(format!("`{name}` is not a constructor of this type"));
                    }
                };
                let tys: Vec<(String, Ty)> = header
                    .iter()
                    .chain(&all[c as usize].fields)
                    .map(|f| (f.name.clone(), f.ty.subst(args)))
                    .collect();
                if tys.is_empty() {
                    return Ok(Value::Ctor(c, Vec::new()));
                }
                Ok(Value::Ctor(c, self.fields(&tys)?))
            }
        }
    }

    pub(super) fn value(&mut self, ty: &Ty) -> P<Value> {
        self.ws();
        match ty {
            Ty::Base(b) => self.base(*b),
            Ty::Data { .. } => self.data(ty),
            Ty::Sum { ctors, sel } => {
                let at = self.pos;
                let name = self.ctor_name()?;
                let c = match ctors.iter().position(|(n, _)| n == name) {
                    Some(c) if sel.as_ref().is_none_or(|s| s.contains(&(c as u32))) => c,
                    _ => {
                        self.pos = at;
                        return self.fail(format!("`{name}` is not a constructor of this type"));
                    }
                };
                let tys = &ctors[c].1;
                if tys.is_empty() {
                    return Ok(Value::Ctor(c as u32, Vec::new()));
                }
                Ok(Value::Ctor(c as u32, self.fields(tys)?))
            }
            Ty::Tag(inner) => {
                let at = self.pos;
                let name = self.ctor_name()?;
                let c = match inner.as_ref() {
                    Ty::Data { def, .. } => self.schema.ty(*def).ctor_index(name),
                    Ty::Sum { ctors, .. } => {
                        ctors.iter().position(|(n, _)| n == name).map(|c| c as u32)
                    }
                    _ => None,
                };
                match c {
                    Some(c) => Ok(Value::Ctor(c, Vec::new())),
                    None => {
                        self.pos = at;
                        self.fail(format!("`{name}` is not a constructor of this type"))
                    }
                }
            }
            Ty::Record(tys) => Ok(Value::Record(self.fields(tys)?)),
            Ty::Unit => {
                self.expect("(")?;
                self.expect(")")?;
                Ok(Value::unit())
            }
            Ty::Key { rel, .. } => self.key(*rel),
            Ty::Set(t) | Ty::NonEmpty(t) => {
                let at = self.pos;
                self.expect("[")?;
                let mut out = BTreeSet::new();
                if !self.eat("]") {
                    loop {
                        out.insert(self.value(t)?);
                        if self.eat("]") {
                            break;
                        }
                        self.expect(",")?;
                    }
                }
                if out.is_empty() && matches!(ty, Ty::NonEmpty(_)) {
                    self.pos = at;
                    return self.fail("an empty set where a nonempty one is required");
                }
                Ok(Value::Set(out))
            }
            Ty::Param(_) | Ty::Group(..) | Ty::Fn(..) | Ty::Change => {
                self.fail("this type has no textual syntax")
            }
        }
    }
}
