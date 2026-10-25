use super::value::{F32, F64, KeyType, Scalar, SumType, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Asc,
    Desc,
}

/// The component types of an index key, each with its direction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeySchema {
    components: Vec<(KeyType, Dir)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    UnexpectedEnd,
    TrailingBytes,
    BadEscape,
    InvalidUtf8,
    NonCanonicalFloat,
    DecimalOutOfRange,
    DateOutOfRange,
    TimeOutOfRange,
    UnknownOrderKey,
    BadVarint,
}

const MICROS_PER_DAY: i64 = 86_400_000_000;
const DECIMAL_LIMIT: u128 = 10_u128.pow(38);

impl KeySchema {
    pub fn new(components: Vec<(KeyType, Dir)>) -> KeySchema {
        KeySchema { components }
    }

    pub fn components(&self) -> &[(KeyType, Dir)] {
        &self.components
    }

    pub fn encode(&self, values: &[Value]) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(values, &mut out);
        out
    }

    /// Append the encoding of `values` to `out`.
    ///
    /// Panics when `values` does not have the schema's component types.
    pub fn encode_into(&self, values: &[Value], out: &mut Vec<u8>) {
        assert_eq!(
            self.components.len(),
            values.len(),
            "key has the wrong number of components"
        );
        for ((ty, dir), v) in self.components.iter().zip(values) {
            encode_value(&mut Writer::new(out, *dir), ty, v);
        }
    }

    /// Returns the key `bytes` encodes, refusing any byte string the
    /// encoding does not produce.
    pub fn decode(&self, bytes: &[u8]) -> Result<Vec<Value>, DecodeError> {
        let mut r = Reader::new(bytes);
        let values = self.decode_from(&mut r)?;
        r.finish()?;
        Ok(values)
    }

    pub(super) fn decode_from(&self, r: &mut Reader) -> Result<Vec<Value>, DecodeError> {
        self.components
            .iter()
            .map(|(ty, dir)| {
                r.mask = mask(*dir);
                let v = decode_value(r, ty);
                r.mask = 0;
                v
            })
            .collect()
    }
}

fn mask(dir: Dir) -> u8 {
    match dir {
        Dir::Asc => 0x00,
        Dir::Desc => 0xFF,
    }
}

struct Writer<'a> {
    out: &'a mut Vec<u8>,
    mask: u8,
}

impl Writer<'_> {
    fn new(out: &mut Vec<u8>, dir: Dir) -> Writer<'_> {
        Writer {
            out,
            mask: mask(dir),
        }
    }

    fn byte(&mut self, b: u8) {
        self.out.push(b ^ self.mask);
    }

    fn bytes(&mut self, bs: &[u8]) {
        let mask = self.mask;
        self.out.extend(bs.iter().map(|b| b ^ mask));
    }

    fn signed<const N: usize>(&mut self, mut be: [u8; N]) {
        be[0] ^= 0x80;
        self.bytes(&be);
    }

    fn escaped(&mut self, bs: &[u8]) {
        for &b in bs {
            self.byte(b);
            if b == 0x00 {
                self.byte(0xFF);
            }
        }
        self.bytes(&[0x00, 0x01]);
    }
}

fn encode_value(w: &mut Writer, ty: &KeyType, v: &Value) {
    match (ty, v) {
        (KeyType::Scalar(s), v) => encode_scalar(w, *s, v),
        (KeyType::Record(tys), Value::Record(vs)) => encode_fields(w, tys, vs),
        (KeyType::Sum(sum), Value::Sum { ctor, fields }) => {
            let c = usize::try_from(*ctor)
                .ok()
                .and_then(|i| sum.ctors().get(i))
                .unwrap_or_else(|| mismatch(ty, v));
            w.bytes(c.order_key.as_bytes());
            w.byte(0x00);
            encode_fields(w, &c.fields, fields);
        }
        _ => mismatch(ty, v),
    }
}

fn encode_fields(w: &mut Writer, tys: &[KeyType], vs: &[Value]) {
    assert_eq!(tys.len(), vs.len(), "value has the wrong number of fields");
    for (ty, v) in tys.iter().zip(vs) {
        encode_value(w, ty, v);
    }
}

fn encode_scalar(w: &mut Writer, s: Scalar, v: &Value) {
    match (s, v) {
        (Scalar::I8, Value::I8(x)) => w.signed(x.to_be_bytes()),
        (Scalar::I16, Value::I16(x)) => w.signed(x.to_be_bytes()),
        (Scalar::I32, Value::I32(x)) | (Scalar::Date, Value::Date(x)) => w.signed(x.to_be_bytes()),
        (Scalar::I64, Value::I64(x))
        | (Scalar::Instant, Value::Instant(x))
        | (Scalar::Time, Value::Time(x))
        | (Scalar::DateTime, Value::DateTime(x))
        | (Scalar::Duration, Value::Duration(x)) => w.signed(x.to_be_bytes()),
        (Scalar::F32, Value::F32(x)) => w.bytes(&f32_order_bits(*x).to_be_bytes()),
        (Scalar::F64, Value::F64(x)) => w.bytes(&f64_order_bits(*x).to_be_bytes()),
        (Scalar::Decimal(_), Value::Decimal(n)) => w.signed(n.to_be_bytes()),
        (Scalar::Text, Value::Text(t)) => w.escaped(t.as_bytes()),
        (Scalar::Bytes, Value::Bytes(b)) | (Scalar::SortKey, Value::SortKey(b)) => w.escaped(b),
        (Scalar::ZonedDateTime, Value::ZonedDateTime { local, zone }) => {
            w.signed(local.to_be_bytes());
            w.escaped(zone.as_bytes());
        }
        (Scalar::Period, Value::Period { months, days }) => {
            w.signed(months.to_be_bytes());
            w.signed(days.to_be_bytes());
        }
        _ => mismatch(&KeyType::Scalar(s), v),
    }
}

fn mismatch(ty: &KeyType, v: &Value) -> ! {
    panic!("value {v:?} does not have key type {ty:?}")
}

fn f32_order_bits(x: F32) -> u32 {
    let bits = x.get().to_bits();
    if bits >> 31 == 1 {
        !bits
    } else {
        bits ^ (1 << 31)
    }
}

fn f64_order_bits(x: F64) -> u64 {
    let bits = x.get().to_bits();
    if bits >> 63 == 1 {
        !bits
    } else {
        bits ^ (1 << 63)
    }
}

fn f32_from_order_bits(o: u32) -> Result<F32, DecodeError> {
    let bits = if o >> 31 == 1 { o ^ (1 << 31) } else { !o };
    let x = f32::from_bits(bits);
    let canonical = F32::new(x);
    if canonical.get().to_bits() == bits {
        Ok(canonical)
    } else {
        Err(DecodeError::NonCanonicalFloat)
    }
}

fn f64_from_order_bits(o: u64) -> Result<F64, DecodeError> {
    let bits = if o >> 63 == 1 { o ^ (1 << 63) } else { !o };
    let x = f64::from_bits(bits);
    let canonical = F64::new(x);
    if canonical.get().to_bits() == bits {
        Ok(canonical)
    } else {
        Err(DecodeError::NonCanonicalFloat)
    }
}

pub(super) struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    mask: u8,
}

impl<'a> Reader<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader {
            bytes,
            pos: 0,
            mask: 0,
        }
    }

    pub(super) fn finish(&self) -> Result<(), DecodeError> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes)
        }
    }

    pub(super) fn byte(&mut self) -> Result<u8, DecodeError> {
        let b = *self.bytes.get(self.pos).ok_or(DecodeError::UnexpectedEnd)?;
        self.pos += 1;
        Ok(b ^ self.mask)
    }

    pub(super) fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let mut a = [0; N];
        for b in &mut a {
            *b = self.byte()?;
        }
        Ok(a)
    }

    fn signed<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let mut a = self.array()?;
        a[0] ^= 0x80;
        Ok(a)
    }

    fn escaped(&mut self) -> Result<Vec<u8>, DecodeError> {
        let mut out = Vec::new();
        loop {
            match self.byte()? {
                0x00 => match self.byte()? {
                    0xFF => out.push(0x00),
                    0x01 => return Ok(out),
                    _ => return Err(DecodeError::BadEscape),
                },
                b => out.push(b),
            }
        }
    }

    fn text(&mut self) -> Result<String, DecodeError> {
        String::from_utf8(self.escaped()?).map_err(|_| DecodeError::InvalidUtf8)
    }

    fn i32(&mut self) -> Result<i32, DecodeError> {
        Ok(i32::from_be_bytes(self.signed()?))
    }

    fn i64(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_be_bytes(self.signed()?))
    }
}

fn decode_value(r: &mut Reader, ty: &KeyType) -> Result<Value, DecodeError> {
    match ty {
        KeyType::Scalar(s) => decode_scalar(r, *s),
        KeyType::Record(tys) => Ok(Value::Record(decode_fields(r, tys)?)),
        KeyType::Sum(sum) => decode_sum(r, sum),
    }
}

fn decode_fields(r: &mut Reader, tys: &[KeyType]) -> Result<Vec<Value>, DecodeError> {
    tys.iter().map(|ty| decode_value(r, ty)).collect()
}

fn decode_sum(r: &mut Reader, sum: &SumType) -> Result<Value, DecodeError> {
    let mut order_key = Vec::new();
    loop {
        match r.byte()? {
            0x00 => break,
            b => order_key.push(b),
        }
    }
    let ctor = sum
        .ctor_of(&order_key)
        .ok_or(DecodeError::UnknownOrderKey)?;
    let fields = decode_fields(r, &sum.ctors()[ctor as usize].fields)?;
    Ok(Value::Sum { ctor, fields })
}

fn decode_scalar(r: &mut Reader, s: Scalar) -> Result<Value, DecodeError> {
    Ok(match s {
        Scalar::I8 => Value::I8(i8::from_be_bytes(r.signed()?)),
        Scalar::I16 => Value::I16(i16::from_be_bytes(r.signed()?)),
        Scalar::I32 => Value::I32(r.i32()?),
        Scalar::I64 => Value::I64(r.i64()?),
        Scalar::F32 => Value::F32(f32_from_order_bits(u32::from_be_bytes(r.array()?))?),
        Scalar::F64 => Value::F64(f64_from_order_bits(u64::from_be_bytes(r.array()?))?),
        Scalar::Decimal(_) => {
            let n = i128::from_be_bytes(r.signed()?);
            if n.unsigned_abs() >= DECIMAL_LIMIT {
                return Err(DecodeError::DecimalOutOfRange);
            }
            Value::Decimal(n)
        }
        Scalar::Text => Value::Text(r.text()?),
        Scalar::Bytes => Value::Bytes(r.escaped()?),
        Scalar::SortKey => Value::SortKey(r.escaped()?),
        Scalar::Instant => Value::Instant(r.i64()?),
        Scalar::Date => {
            let days = r.i32()?;
            if i64::from(days).checked_mul(MICROS_PER_DAY).is_none() {
                return Err(DecodeError::DateOutOfRange);
            }
            Value::Date(days)
        }
        Scalar::Time => {
            let t = r.i64()?;
            if !(0..MICROS_PER_DAY).contains(&t) {
                return Err(DecodeError::TimeOutOfRange);
            }
            Value::Time(t)
        }
        Scalar::DateTime => Value::DateTime(r.i64()?),
        Scalar::ZonedDateTime => {
            let local = r.i64()?;
            let zone = r.text()?;
            Value::ZonedDateTime { local, zone }
        }
        Scalar::Duration => Value::Duration(r.i64()?),
        Scalar::Period => {
            let months = r.i32()?;
            let days = r.i32()?;
            Value::Period { months, days }
        }
    })
}
