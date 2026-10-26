use std::fmt;

use super::{Ctor, Form, KeyForm, Scalar, Textual};
use crate::calendar::{MICROS_PER_DAY, at_day, date_in_range, days_from_civil, days_in_month};
use crate::lex;
use crate::types::Base;

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

/// Reads `text`, the textual syntax of one value, at form `form`.
pub fn parse<V: Textual>(text: &str, form: &Form) -> P<V> {
    let mut r = Reader::new(text);
    let v = r.value(form)?;
    r.end()?;
    Ok(v)
}

/// Returns the 16 bytes of the UUID `t` writes in the 8-4-4-4-12 form, in either case.
pub fn parse_uuid(t: &str) -> Option<Vec<u8>> {
    let b = t.as_bytes();
    if b.len() != 36 {
        return None;
    }
    let mut out = Vec::with_capacity(16);
    let mut i = 0;
    while i < 36 {
        if matches!(i, 8 | 13 | 18 | 23) {
            if b[i] != b'-' {
                return None;
            }
            i += 1;
            continue;
        }
        let hi = char::from(b[i]).to_digit(16)?;
        let lo = char::from(b[i + 1]).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
        i += 2;
    }
    Some(out)
}

/// Returns `f` with `-0.0` made `0.0` and every NaN made the one NaN.
fn canon64(f: f64) -> f64 {
    if f.is_nan() {
        f64::NAN
    } else if f == 0.0 {
        0.0
    } else {
        f
    }
}

fn canon32(f: f32) -> f32 {
    if f.is_nan() {
        f32::NAN
    } else if f == 0.0 {
        0.0
    } else {
        f
    }
}

pub(super) struct Reader<'s> {
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
    pub(super) fn new(src: &'s str) -> Reader<'s> {
        Reader { src, pos: 0 }
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

    fn integer(&mut self, b: Base) -> P<i64> {
        let neg = self.sign();
        let d = self.some_digits()?.trim_start_matches('0');
        let v = d.parse::<i128>().unwrap_or(0);
        let v = if neg { -v } else { v };
        let in_range = |(lo, hi): (i64, i64)| i128::from(lo) <= v && v <= i128::from(hi);
        match b.int_range().filter(|r| d.len() <= 20 && in_range(*r)) {
            Some(_) => Ok(v as i64),
            None => self.fail("an integer out of range of its type"),
        }
    }

    fn decimal(&mut self, s: u8) -> P<i128> {
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
        Ok(if neg { -v } else { v })
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
            _ => i64::from(self.fixed_digits(4)?),
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
        Ok(((i64::from(h) * 60 + i64::from(m)) * 60 + i64::from(s)) * 1_000_000 + micros)
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
        let v = if neg { -n } else { n } * i128::from(unit);
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

    fn period(&mut self) -> P<(i32, i32)> {
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
        Ok((months, days))
    }

    fn zone<V: Textual>(&mut self) -> P<&'s str> {
        if !self.glued("[") {
            return self.fail("expected `[`");
        }
        let start = self.pos;
        let Some(len) = self.rest().find(']') else {
            return self.fail("expected `]`");
        };
        let name = &self.src[start..start + len];
        if !V::zone_exists(name) {
            return self.fail(format!("no zone `{name}` in the zone rules"));
        }
        self.pos += len + 1;
        Ok(name)
    }

    fn base<V: Textual>(&mut self, b: Base) -> P<V> {
        let text;
        let bytes;
        let s = match b {
            Base::I8 | Base::I16 | Base::I32 | Base::I64 => Scalar::Int(self.integer(b)?),
            Base::Decimal(s) => Scalar::Decimal(self.decimal(s)?),
            Base::F64 => Scalar::F64(canon64(self.float()?)),
            Base::F32 => Scalar::F32(canon32(self.float32()?)),
            Base::Text => {
                text = self.text()?;
                Scalar::Text(&text)
            }
            Base::Bytes | Base::SortKey => {
                bytes = self.hex()?;
                Scalar::Bytes(&bytes)
            }
            Base::Instant => {
                let t = self.datetime()?;
                if !self.glued("Z") {
                    return self.fail("expected `Z`");
                }
                Scalar::Instant(t)
            }
            Base::Date => Scalar::Date(self.date()?),
            Base::Time => Scalar::Time(self.time()?),
            Base::DateTime => Scalar::DateTime(self.datetime()?),
            Base::ZonedDateTime => {
                let local = self.datetime()?;
                Scalar::Zoned {
                    local,
                    zone: self.zone::<V>()?,
                }
            }
            Base::Duration => Scalar::Duration(self.duration()?),
            Base::Period => {
                let (months, days) = self.period()?;
                Scalar::Period { months, days }
            }
        };
        Ok(V::of_scalar(s, b))
    }

    fn fields<'f, V: Textual>(&mut self, forms: &'f [(String, Form)]) -> P<Vec<(&'f str, V)>> {
        self.expect("{")?;
        self.field_list(forms, "}")
    }

    /// Reads `f: v, ...` up to and including `close`, with every field of `forms` once, in any
    /// order, and returns the values in the order of `forms`.
    fn field_list<'f, V: Textual>(
        &mut self,
        forms: &'f [(String, Form)],
        close: &str,
    ) -> P<Vec<(&'f str, V)>> {
        let mut got: Vec<Option<V>> = (0..forms.len()).map(|_| None).collect();
        if !self.eat(close) {
            loop {
                let at = self.pos;
                let name = self.ident()?;
                let Some(i) = forms.iter().position(|(n, _)| n == name) else {
                    self.pos = at;
                    return self.fail(format!("no field `{name}`"));
                };
                if got[i].is_some() {
                    self.pos = at;
                    return self.fail(format!("field `{name}` given twice"));
                }
                self.expect(":")?;
                got[i] = Some(self.value(&forms[i].1)?);
                if self.eat(close) {
                    break;
                }
                self.expect(",")?;
            }
        }
        forms
            .iter()
            .zip(got)
            .map(|((n, _), v)| match v {
                Some(v) => Ok((n.as_str(), v)),
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

    /// Reads a constructor name and returns its index, which `allowed` must hold.
    fn ctor_index<'n>(
        &mut self,
        names: impl Iterator<Item = &'n str>,
        allowed: Option<&[u32]>,
    ) -> P<u32> {
        let at = self.pos;
        let name = self.ctor_name()?;
        let found = names
            .enumerate()
            .find(|(_, n)| *n == name)
            .map(|(c, _)| c as u32);
        match found {
            Some(c) if allowed.is_none_or(|s| s.contains(&c)) => Ok(c),
            _ => {
                self.pos = at;
                self.fail(format!("`{name}` is not a constructor of this type"))
            }
        }
    }

    fn sum<V: Textual>(&mut self, ctors: &[Ctor], allowed: Option<&[u32]>) -> P<V> {
        let c = self.ctor_index(ctors.iter().map(|c| c.name.as_str()), allowed)?;
        let fields = &ctors[c as usize].fields;
        if fields.is_empty() {
            return Ok(V::of_ctor(c, Vec::new()));
        }
        Ok(V::of_ctor(c, self.fields(fields)?))
    }

    fn key<V: Textual>(&mut self, rel: &str, value: &KeyForm) -> P<V> {
        let at = self.pos;
        if self.name()? != rel {
            self.pos = at;
            return self.fail(format!("expected a key of `{rel}`"));
        }
        if !self.glued(".key(") {
            return self.fail("expected `.key(`");
        }
        let v = match value {
            KeyForm::One(f) => {
                let v = self.value(f)?;
                self.expect(")")?;
                v
            }
            KeyForm::Fields(fs) => V::of_record(self.field_list(fs, ")")?),
        };
        Ok(V::of_key(v))
    }

    fn uuid<V: Textual>(&mut self) -> P<V> {
        self.expect("uuid")?;
        if !self.glued("(") {
            return self.fail("expected `(`");
        }
        let at = self.pos;
        let t = self.text()?;
        let Some(b) = parse_uuid(&t) else {
            self.pos = at;
            return self.fail("not a UUID in the 8-4-4-4-12 form");
        };
        self.expect(")")?;
        Ok(V::of_scalar(Scalar::Bytes(&b), Base::Bytes))
    }

    fn span<V: Textual>(&mut self, point: &Form) -> P<V> {
        let at = self.pos;
        self.expect("span")?;
        if !self.glued("(") {
            return self.fail("expected `(`");
        }
        let start: V = self.value(point)?;
        self.expect(",")?;
        let end = self.value(point)?;
        self.expect(")")?;
        if start > end {
            self.pos = at;
            return self.fail("a span whose start is after its end");
        }
        Ok(V::of_ctor(0, vec![("start", start), ("end", end)]))
    }

    fn set<V: Textual>(&mut self, elem: &Form, nonempty: bool) -> P<V> {
        let at = self.pos;
        self.expect("[")?;
        let mut out = Vec::new();
        if !self.eat("]") {
            loop {
                out.push(self.value(elem)?);
                if self.eat("]") {
                    break;
                }
                self.expect(",")?;
            }
        }
        if out.is_empty() && nonempty {
            self.pos = at;
            return self.fail("an empty set where a nonempty one is required");
        }
        Ok(V::of_set(out))
    }

    pub(super) fn value<V: Textual>(&mut self, form: &Form) -> P<V> {
        self.ws();
        match form {
            Form::Base(b) => self.base(*b),
            Form::Uuid => self.uuid(),
            Form::Newtype { name, base } => {
                let at = self.pos;
                if self.name()? != name {
                    self.pos = at;
                    return self.fail(format!("expected `{name}(`"));
                }
                if !self.glued("(") {
                    return self.fail("expected `(`");
                }
                let v = self.value(base)?;
                self.expect(")")?;
                Ok(v)
            }
            Form::Span(point) => self.span(point),
            Form::Sum { ctors, allowed } => self.sum(ctors, allowed.as_deref()),
            Form::Tag(names) => {
                let c = self.ctor_index(names.iter().map(String::as_str), None)?;
                Ok(V::of_ctor(c, Vec::new()))
            }
            Form::Record(fs) => Ok(V::of_record(self.fields(fs)?)),
            Form::Unit => {
                self.expect("(")?;
                self.expect(")")?;
                Ok(V::of_record(Vec::new()))
            }
            Form::Key { rel, value } => self.key(rel, value),
            Form::Set { elem, nonempty } => self.set(elem, *nonempty),
            Form::Opaque => self.fail("this type has no textual syntax"),
        }
    }
}
