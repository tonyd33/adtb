use std::collections::BTreeSet;
use std::fmt::Write;
use std::sync::Arc;

use adtb_lang::schema::{Generator, Schema, TypeDefKind};
use adtb_lang::types::{Base, RelId, Ty};

use super::super::Error;
use super::super::types::{ctor_set, ctors, repr};
use super::num::{decimal_text, f32_text, f64_text};
use super::time::{
    DAY_MAX, DAY_MIN, UNITS, at_day, date_text, datetime_text, days_from_civil, days_in_month,
    duration_text, time_text, zone,
};
use super::unicode::nfc;
use super::{F32, F64, Val, Width};

/// The shape of the values of a type, as their textual forms need it.
#[derive(Clone, Debug)]
pub enum Form {
    Int(Width),
    Decimal(u8),
    F32,
    F64,
    Text,
    Bytes,
    SortKey,
    Instant,
    Date,
    Time,
    DateTime,
    Zoned,
    Duration,
    Period,
    Bool,
    Uuid,
    /// A newtype other than `Uuid`, by name.
    Newtype(String, Box<Form>),
    /// A sum's constructors, by position; `None` for those the type excludes.
    Sum(Vec<Option<Ctor>>),
    /// An anonymous record, its fields sorted by name.
    Record(Vec<(String, Form)>),
    Key {
        rel: String,
        module: String,
        value: KeyForm,
    },
    Set {
        elem: Box<Form>,
        nonempty: bool,
    },
    Span(Box<Form>),
}

#[derive(Clone, Debug)]
pub struct Ctor {
    pub name: String,
    /// The fields in the order a `Val::Ctor` holds them.
    pub fields: Vec<(String, Form)>,
}

#[derive(Clone, Debug)]
pub enum KeyForm {
    One(Box<Form>),
    /// The fields of a composite key, sorted by name.
    Fields(Vec<(String, Form)>),
}

fn unsupported<T>(ty: &Ty) -> Result<T, Error> {
    Err(Error::Unsupported(format!("the textual form of {ty:?}")))
}

impl Form {
    pub fn of(schema: &Schema, ty: &Ty) -> Result<Form, Error> {
        if let Ty::Data { def, args, .. } = ty {
            let d = schema.ty(*def);
            if *def == schema.prelude_type("Uuid") {
                return Ok(Form::Uuid);
            }
            if *def == schema.prelude_type("Bool") {
                return Ok(Form::Bool);
            }
            if *def == schema.prelude_type("Span") {
                return Ok(Form::Span(Box::new(Form::of(schema, &args[0])?)));
            }
            if let TypeDefKind::Newtype { base, .. } = &d.kind {
                return Ok(Form::Newtype(
                    d.name.clone(),
                    Box::new(Form::of(schema, &base.subst(args))?),
                ));
            }
        }
        Ok(match repr(schema, ty) {
            Ty::Base(b) => match b {
                Base::I8 => Form::Int(Width::I8),
                Base::I16 => Form::Int(Width::I16),
                Base::I32 => Form::Int(Width::I32),
                Base::I64 => Form::Int(Width::I64),
                Base::Decimal(s) => Form::Decimal(s),
                Base::F32 => Form::F32,
                Base::F64 => Form::F64,
                Base::Text => Form::Text,
                Base::Bytes => Form::Bytes,
                Base::SortKey => Form::SortKey,
                Base::Instant => Form::Instant,
                Base::Date => Form::Date,
                Base::Time => Form::Time,
                Base::DateTime => Form::DateTime,
                Base::ZonedDateTime => Form::Zoned,
                Base::Duration => Form::Duration,
                Base::Period => Form::Period,
            },
            t @ (Ty::Data { .. } | Ty::Sum { .. } | Ty::Tag(_)) => {
                let names: Vec<String> = match &t {
                    Ty::Data { def, .. } => schema
                        .ty(*def)
                        .ctors()
                        .iter()
                        .map(|c| c.name.clone())
                        .collect(),
                    Ty::Sum { ctors, .. } => ctors.iter().map(|(n, _)| n.clone()).collect(),
                    Ty::Tag(inner) => match repr(schema, inner) {
                        Ty::Data { def, .. } => schema
                            .ty(def)
                            .ctors()
                            .iter()
                            .map(|c| c.name.clone())
                            .collect(),
                        Ty::Sum { ctors, .. } => ctors.iter().map(|(n, _)| n.clone()).collect(),
                        _ => return unsupported(ty),
                    },
                    _ => unreachable!(),
                };
                let fields = ctors(schema, &t).expect("a sum has constructors");
                let allowed = ctor_set(schema, &t);
                let mut out = Vec::with_capacity(names.len());
                for (i, (name, fs)) in names.into_iter().zip(fields).enumerate() {
                    if !allowed.contains(&(i as u32)) {
                        out.push(None);
                        continue;
                    }
                    let fields = fs
                        .iter()
                        .map(|(n, t)| Ok((n.clone(), Form::of(schema, t)?)))
                        .collect::<Result<_, Error>>()?;
                    out.push(Some(Ctor { name, fields }));
                }
                Form::Sum(out)
            }
            Ty::Record(fs) => Form::Record(
                fs.iter()
                    .map(|(n, t)| Ok((n.clone(), Form::of(schema, t)?)))
                    .collect::<Result<_, Error>>()?,
            ),
            Ty::Key { rel, .. } => key_form(schema, rel)?,
            Ty::Set(e) => Form::Set {
                elem: Box::new(Form::of(schema, &e)?),
                nonempty: false,
            },
            Ty::NonEmpty(e) => Form::Set {
                elem: Box::new(Form::of(schema, &e)?),
                nonempty: true,
            },
            _ => return unsupported(ty),
        })
    }
}

fn key_form(schema: &Schema, rel: RelId) -> Result<Form, Error> {
    let r = schema.relation(rel);
    let def = schema.ty(r.elem_def);
    let field = |name: &str| -> Result<Form, Error> {
        let f = def
            .field(0, name)
            .unwrap_or_else(|| panic!("`{}` has key field `{name}`", r.name));
        match &f.ty {
            Ty::Key { rel: own, .. } if *own == rel => match r.generate {
                Some(Generator::Uuidv7) => Ok(Form::Uuid),
                _ => Ok(Form::Int(Width::I64)),
            },
            t => Form::of(schema, &t.subst(r.elem_args())),
        }
    };
    let value = match r.key.as_slice() {
        [one] => KeyForm::One(Box::new(field(one)?)),
        many => {
            let mut names = many.to_vec();
            names.sort();
            KeyForm::Fields(
                names
                    .iter()
                    .map(|n| Ok((n.clone(), field(n)?)))
                    .collect::<Result<_, Error>>()?,
            )
        }
    };
    Ok(Form::Key {
        rel: r.name.clone(),
        module: schema.module(r.module).name.clone(),
        value,
    })
}

fn hex(out: &mut String, b: &[u8]) {
    out.push_str("0x");
    for x in b {
        write!(out, "{x:02x}").expect("a String accepts every write");
    }
}

fn uuid_text(b: &[u8]) -> String {
    let mut s = String::with_capacity(36);
    for (i, x) in b.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            s.push('-');
        }
        write!(s, "{x:02x}").expect("a String accepts every write");
    }
    s
}

/// Returns the 16 bytes of the UUID `t` writes in the canonical form, either case.
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
        let hi = (b[i] as char).to_digit(16)?;
        let lo = (b[i + 1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
        i += 2;
    }
    Some(out)
}

fn bytes(v: &Val) -> &[u8] {
    match v {
        Val::Bytes(b) | Val::SortKey(b) => b,
        other => panic!("expected bytes, found {other:?}"),
    }
}

fn quote(out: &mut String, t: &str) {
    out.push('"');
    for c in t.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                write!(out, "\\u{{{:x}}}", c as u32).expect("a String accepts every write");
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Returns `text(v)` for a value of a number, a time type, `Uuid` or a newtype over `Text`.
pub fn text(v: &Val, f: &Form) -> String {
    match (f, v) {
        (Form::Newtype(_, base), v) if matches!(**base, Form::Text) => match v {
            Val::Text(t) => t.to_string(),
            other => panic!("expected text, found {other:?}"),
        },
        (Form::Uuid, v) => uuid_text(bytes(v)),
        _ => print(v, f),
    }
}

/// Returns the canonical printed form of `v`.
pub fn print(v: &Val, f: &Form) -> String {
    let mut out = String::new();
    write_val(&mut out, v, f);
    out
}

fn write_fields(out: &mut String, fields: &[(String, Form)], vs: &[Val]) {
    if fields.is_empty() {
        out.push_str("{}");
        return;
    }
    out.push_str("{ ");
    for (i, ((n, f), v)) in fields.iter().zip(vs).enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(n);
        out.push_str(": ");
        write_val(out, v, f);
    }
    out.push_str(" }");
}

fn write_val(out: &mut String, v: &Val, f: &Form) {
    match (f, v) {
        (Form::Int(_), Val::Int(i)) => {
            write!(out, "{i}").expect("a String accepts every write");
        }
        (Form::Decimal(s), Val::Decimal(d)) => out.push_str(&decimal_text(*d, *s)),
        (Form::F32, Val::F32(x)) => out.push_str(&f32_text(x.get())),
        (Form::F64, Val::F64(x)) => out.push_str(&f64_text(x.get())),
        (Form::Text, Val::Text(t)) => quote(out, t),
        (Form::Bytes | Form::SortKey, v) => hex(out, bytes(v)),
        (Form::Instant, Val::Instant(i)) => {
            out.push_str(&datetime_text(*i));
            out.push('Z');
        }
        (Form::Date, Val::Date(d)) => out.push_str(&date_text(i64::from(*d))),
        (Form::Time, Val::Time(t)) => out.push_str(&time_text(*t)),
        (Form::DateTime, Val::DateTime(t)) => out.push_str(&datetime_text(*t)),
        (Form::Zoned, Val::Zoned { local, zone }) => {
            write!(out, "{}[{zone}]", datetime_text(*local)).expect("a String accepts every write");
        }
        (Form::Duration, Val::Duration(d)) => out.push_str(&duration_text(*d)),
        (Form::Period, Val::Period { months, days }) => {
            write!(out, "P{months}M{days}D").expect("a String accepts every write");
        }
        (Form::Bool, v) => out.push_str(if v.truth() { "True" } else { "False" }),
        (Form::Uuid, v) => {
            write!(out, "uuid(\"{}\")", uuid_text(bytes(v))).expect("a String accepts every write");
        }
        (Form::Newtype(name, base), v) => {
            out.push_str(name);
            out.push('(');
            write_val(out, v, base);
            out.push(')');
        }
        (Form::Sum(cs), Val::Ctor(c, vs)) => {
            let ctor = cs[*c as usize]
                .as_ref()
                .expect("a value's constructor is one of its type's");
            out.push_str(&ctor.name);
            if !ctor.fields.is_empty() {
                out.push(' ');
                write_fields(out, &ctor.fields, vs);
            }
        }
        (Form::Record(fs), Val::Record(vs)) => write_fields(out, fs, vs),
        (Form::Key { rel, value, .. }, v) => {
            out.push_str(rel);
            out.push_str(".key(");
            match value {
                KeyForm::One(f) => write_val(out, v, f),
                KeyForm::Fields(fs) => {
                    for (i, ((n, f), x)) in fs.iter().zip(v.fields()).enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        out.push_str(n);
                        out.push_str(": ");
                        write_val(out, x, f);
                    }
                }
            }
            out.push(')');
        }
        (Form::Set { elem, .. }, Val::Set(s)) => {
            out.push('[');
            for (i, x) in s.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_val(out, x, elem);
            }
            out.push(']');
        }
        (Form::Span(a), v) => {
            let fs = v.fields();
            out.push_str("span(");
            write_val(out, &fs[0], a);
            out.push_str(", ");
            write_val(out, &fs[1], a);
            out.push(')');
        }
        (f, v) => panic!("{v:?} is not a value of form {f:?}"),
    }
}

/// Returns the value text `t` reads as at form `f`, or `None` when it does not read as one.
pub fn parse(t: &str, f: &Form) -> Option<Val> {
    let mut r = Reader { s: t, at: 0 };
    let v = r.value(f)?;
    r.ws();
    (r.at == t.len()).then_some(v)
}

struct Reader<'a> {
    s: &'a str,
    at: usize,
}

impl<'a> Reader<'a> {
    fn src(&self) -> &'a str {
        self.s
    }

    fn rest(&self) -> &'a str {
        &self.src()[self.at..]
    }

    fn peek(&self) -> Option<u8> {
        self.s.as_bytes().get(self.at).copied()
    }

    fn ws(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_whitespace()) {
            self.at += 1;
        }
    }

    /// Consumes `p` when the text continues with it.
    fn eat(&mut self, p: &str) -> bool {
        if self.rest().starts_with(p) {
            self.at += p.len();
            true
        } else {
            false
        }
    }

    /// Skips whitespace, then consumes `p` when the text continues with it.
    fn token(&mut self, p: &str) -> Option<()> {
        self.ws();
        self.eat(p).then_some(())
    }

    fn take_while(&mut self, f: impl Fn(u8) -> bool) -> &'a str {
        let start = self.at;
        while self.peek().is_some_and(&f) {
            self.at += 1;
        }
        &self.src()[start..self.at]
    }

    fn ident(&mut self) -> Option<&'a str> {
        self.ws();
        let start = self.at;
        if !self
            .peek()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        {
            return None;
        }
        self.take_while(|b| b.is_ascii_alphanumeric() || b == b'_');
        Some(&self.src()[start..self.at])
    }

    fn digits(&mut self) -> Option<&'a str> {
        let d = self.take_while(|b| b.is_ascii_digit());
        (!d.is_empty()).then_some(d)
    }

    /// Reads exactly `n` digits.
    fn fixed(&mut self, n: usize) -> Option<u32> {
        let d = self.rest().get(..n)?;
        if !d.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        self.at += n;
        d.parse().ok()
    }

    /// Reads an optional `-`, then the parts of a numeric literal: integer digits, fractional
    /// digits after a `.`, and an exponent after an `e`.
    fn number(&mut self) -> Option<Number<'a>> {
        self.ws();
        let neg = self.eat("-");
        self.ws();
        let start = self.at;
        self.digits()?;
        let int_end = self.at;
        let mut frac = None;
        if self.peek() == Some(b'.') {
            self.at += 1;
            let f = self.at;
            self.digits()?;
            frac = Some(f..self.at);
        }
        let mut exp = false;
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            self.digits()?;
            exp = true;
        }
        Some(Number {
            neg,
            int: &self.src()[start..int_end],
            frac: frac.map(|r| &self.src()[r]),
            literal: &self.src()[start..self.at],
            exp,
        })
    }

    fn int(&mut self, w: Width) -> Option<i64> {
        let n = self.number()?;
        if n.frac.is_some() || n.exp {
            return None;
        }
        let digits = n.int.trim_start_matches('0');
        if digits.len() > 20 {
            return None;
        }
        let m: i128 = if digits.is_empty() {
            0
        } else {
            digits.parse().ok()?
        };
        w.fit(if n.neg { -m } else { m })
    }

    fn decimal(&mut self, s: u8) -> Option<i128> {
        let n = self.number()?;
        let frac = n.frac.unwrap_or("");
        if n.exp || frac.len() > usize::from(s) {
            return None;
        }
        let digits = format!("{}{frac:0<width$}", n.int, width = usize::from(s));
        let digits = digits.trim_start_matches('0');
        if digits.len() > 38 {
            return None;
        }
        let m: i128 = if digits.is_empty() {
            0
        } else {
            digits.parse().ok()?
        };
        Some(if n.neg { -m } else { m })
    }

    /// Reads a float literal, `inf` or `NaN`, each after an optional `-`, at `F32` when `single`.
    fn float(&mut self, single: bool) -> Option<f64> {
        self.ws();
        let save = self.at;
        let neg = self.eat("-");
        self.ws();
        for (word, x) in [("inf", f64::INFINITY), ("NaN", f64::NAN)] {
            if self.eat(word) {
                return Some(if neg { -x } else { x });
            }
        }
        self.at = save;
        let n = self.number()?;
        let x = if single {
            f64::from(n.literal.parse::<f32>().ok()?)
        } else {
            n.literal.parse::<f64>().ok()?
        };
        if x.is_infinite() {
            return None;
        }
        Some(if n.neg { -x } else { x })
    }

    fn text(&mut self) -> Option<String> {
        self.token("\"")?;
        let mut out = String::new();
        let mut cs = self.rest().char_indices();
        loop {
            let (i, c) = cs.next()?;
            match c {
                '"' => {
                    self.at += i + 1;
                    return Some(nfc(&out));
                }
                '\\' => {
                    let (_, e) = cs.next()?;
                    match e {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            if cs.next()?.1 != '{' {
                                return None;
                            }
                            let mut code = String::new();
                            loop {
                                match cs.next()?.1 {
                                    '}' => break,
                                    h if h.is_ascii_hexdigit() => code.push(h),
                                    _ => return None,
                                }
                            }
                            if code.is_empty() || code.len() > 8 {
                                return None;
                            }
                            out.push(char::from_u32(u32::from_str_radix(&code, 16).ok()?)?);
                        }
                        _ => return None,
                    }
                }
                c => out.push(c),
            }
        }
    }

    fn hex(&mut self) -> Option<Vec<u8>> {
        self.token("0x")?;
        let h = self.take_while(|b| b.is_ascii_hexdigit());
        if !h.len().is_multiple_of(2) {
            return None;
        }
        (0..h.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&h[i..i + 2], 16).ok())
            .collect()
    }

    /// Reads a date as its days since 1970-01-01.
    fn date(&mut self) -> Option<i64> {
        self.ws();
        let y: i64 = match self.peek() {
            Some(b @ (b'+' | b'-')) => {
                self.at += 1;
                let d = self.digits()?;
                if d.len() < 5 {
                    return None;
                }
                let y: i64 = d.parse().ok()?;
                if b == b'-' { -y } else { y }
            }
            _ => i64::from(self.fixed(4)?),
        };
        self.eat("-").then_some(())?;
        let m = self.fixed(2)?;
        self.eat("-").then_some(())?;
        let d = self.fixed(2)?;
        if !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) {
            return None;
        }
        if y.unsigned_abs() > 1 << 40 {
            return None;
        }
        let days = days_from_civil(y, m, d);
        (DAY_MIN..=DAY_MAX).contains(&days).then_some(days)
    }

    /// Reads a time of day as its microseconds since midnight.
    fn time(&mut self) -> Option<i64> {
        self.ws();
        let h = self.fixed(2)?;
        self.eat(":").then_some(())?;
        let m = self.fixed(2)?;
        self.eat(":").then_some(())?;
        let s = self.fixed(2)?;
        if h > 23 || m > 59 || s > 59 {
            return None;
        }
        let mut us = 0;
        if self.eat(".") {
            let f = self.digits()?;
            if f.len() > 6 {
                return None;
            }
            us = format!("{f:0<6}").parse::<i64>().ok()?;
        }
        Some((i64::from(h) * 3600 + i64::from(m) * 60 + i64::from(s)) * 1_000_000 + us)
    }

    fn datetime(&mut self) -> Option<i64> {
        let d = self.date()?;
        self.eat("T").then_some(())?;
        let t = self.time()?;
        at_day(d, t)
    }

    fn duration(&mut self) -> Option<i64> {
        self.ws();
        let neg = self.eat("-");
        self.ws();
        let n: i64 = self.digits()?.parse().ok()?;
        let unit = self.take_while(|b| b.is_ascii_lowercase());
        let per = UNITS.iter().find(|(u, _)| *u == unit)?.1;
        let d = n.checked_mul(per)?;
        Some(if neg { -d } else { d })
    }

    fn signed32(&mut self) -> Option<i32> {
        let neg = self.eat("-");
        let n: i64 = self.digits()?.parse().ok()?;
        i32::try_from(if neg { -n } else { n }).ok()
    }

    fn period(&mut self) -> Option<Val> {
        self.token("P")?;
        let months = self.signed32()?;
        self.eat("M").then_some(())?;
        let days = self.signed32()?;
        self.eat("D").then_some(())?;
        Some(Val::Period { months, days })
    }

    /// Reads `{ f: v, ... }` with each of `fields` once, in any order; returns the values in the
    /// order of `fields`.
    fn fields(&mut self, fields: &[(String, Form)]) -> Option<Vec<Val>> {
        self.token("{")?;
        let mut vals: Vec<Option<Val>> = vec![None; fields.len()];
        if fields.is_empty() {
            self.token("}")?;
            return Some(Vec::new());
        }
        for i in 0..fields.len() {
            if i > 0 {
                self.token(",")?;
            }
            self.named(fields, &mut vals)?;
        }
        self.token("}")?;
        vals.into_iter().collect()
    }

    /// Reads `f: v` for one of `fields` not yet read.
    fn named(&mut self, fields: &[(String, Form)], vals: &mut [Option<Val>]) -> Option<()> {
        let name = self.ident()?.to_string();
        let i = fields.iter().position(|(n, _)| *n == name)?;
        if vals[i].is_some() {
            return None;
        }
        self.token(":")?;
        vals[i] = Some(self.value(&fields[i].1)?);
        Some(())
    }

    #[expect(
        clippy::cognitive_complexity,
        reason = "one arm per form; the newtype, sum and record arms would have to be functions"
    )]
    fn value(&mut self, f: &Form) -> Option<Val> {
        Some(match f {
            Form::Int(w) => Val::Int(self.int(*w)?),
            Form::Decimal(s) => Val::Decimal(self.decimal(*s)?),
            Form::F32 => Val::F32(F32::new(self.float(true)? as f32)),
            Form::F64 => Val::F64(F64::new(self.float(false)?)),
            Form::Text => Val::Text(Arc::from(self.text()?)),
            Form::Bytes => Val::Bytes(Arc::from(self.hex()?)),
            Form::SortKey => Val::SortKey(Arc::from(self.hex()?)),
            Form::Instant => {
                let t = self.datetime()?;
                self.eat("Z").then_some(())?;
                Val::Instant(t)
            }
            Form::Date => Val::Date(i32::try_from(self.date()?).ok()?),
            Form::Time => Val::Time(self.time()?),
            Form::DateTime => Val::DateTime(self.datetime()?),
            Form::Zoned => {
                let local = self.datetime()?;
                self.eat("[").then_some(())?;
                let name = self.take_while(|b| b != b']').to_string();
                self.eat("]").then_some(())?;
                zone(&name).ok()?;
                Val::Zoned {
                    local,
                    zone: Arc::from(name),
                }
            }
            Form::Duration => Val::Duration(self.duration()?),
            Form::Period => self.period()?,
            Form::Bool => match self.ident()? {
                "True" | "true" => Val::bool(true),
                "False" | "false" => Val::bool(false),
                _ => return None,
            },
            Form::Uuid => {
                (self.ident()? == "uuid").then_some(())?;
                self.token("(")?;
                let t = self.text()?;
                self.token(")")?;
                Val::Bytes(Arc::from(parse_uuid(&t)?))
            }
            Form::Newtype(name, base) => {
                (self.ident()? == name).then_some(())?;
                self.token("(")?;
                let v = self.value(base)?;
                self.token(")")?;
                v
            }
            Form::Sum(cs) => {
                let name = self.ident()?;
                let (c, ctor) = cs
                    .iter()
                    .enumerate()
                    .find_map(|(i, c)| c.as_ref().filter(|c| c.name == name).map(|c| (i, c)))?;
                let vals = if ctor.fields.is_empty() {
                    Vec::new()
                } else {
                    self.fields(&ctor.fields)?
                };
                Val::ctor(c as u32, vals)
            }
            Form::Record(fs) => Val::record(self.fields(fs)?),
            Form::Key { rel, module, value } => {
                let mut name = self.ident()?;
                if name == module && self.rest().starts_with("::") {
                    self.at += 2;
                    name = self.ident()?;
                }
                (name == rel).then_some(())?;
                self.eat(".key").then_some(())?;
                self.token("(")?;
                let v = match value {
                    KeyForm::One(f) => self.value(f)?,
                    KeyForm::Fields(fs) => {
                        let mut vals = vec![None; fs.len()];
                        for i in 0..fs.len() {
                            if i > 0 {
                                self.token(",")?;
                            }
                            self.named(fs, &mut vals)?;
                        }
                        Val::record(vals.into_iter().collect::<Option<_>>()?)
                    }
                };
                self.token(")")?;
                v
            }
            Form::Set { elem, nonempty } => {
                self.token("[")?;
                let mut out = BTreeSet::new();
                self.ws();
                if !self.eat("]") {
                    loop {
                        out.insert(self.value(elem)?);
                        if self.token("]").is_some() {
                            break;
                        }
                        self.token(",")?;
                    }
                }
                if *nonempty && out.is_empty() {
                    return None;
                }
                Val::set(out)
            }
            Form::Span(a) => {
                (self.ident()? == "span").then_some(())?;
                self.token("(")?;
                let s = self.value(a)?;
                self.token(",")?;
                let e = self.value(a)?;
                self.token(")")?;
                if s > e {
                    return None;
                }
                Val::ctor(0, vec![s, e])
            }
        })
    }
}

struct Number<'a> {
    neg: bool,
    int: &'a str,
    frac: Option<&'a str>,
    /// The literal without its sign.
    literal: &'a str,
    exp: bool,
}
