use std::cmp::Ordering;
use std::collections::BTreeSet;

use super::codec::Reader;
use super::*;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }

    fn pick<T: Clone>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len())].clone()
    }
}

const ORDERED: [Scalar; 15] = [
    Scalar::I8,
    Scalar::I16,
    Scalar::I32,
    Scalar::I64,
    Scalar::F32,
    Scalar::F64,
    Scalar::Decimal(2),
    Scalar::Text,
    Scalar::Bytes,
    Scalar::SortKey,
    Scalar::Instant,
    Scalar::Date,
    Scalar::Time,
    Scalar::DateTime,
    Scalar::Duration,
];

const UNORDERED: [Scalar; 2] = [Scalar::ZonedDateTime, Scalar::Period];

const TEXT_CHARS: [char; 12] = [
    '\0',
    '\u{1}',
    'a',
    'b',
    '\u{7F}',
    '\u{80}',
    '\u{FF}',
    '\u{7FF}',
    '\u{800}',
    '\u{FFFF}',
    '\u{10000}',
    '\u{10FFFF}',
];

const BYTES: [u8; 7] = [0x00, 0x01, 0x02, 0x7F, 0x80, 0xFE, 0xFF];

const MAX_DECIMAL: i128 = 10_i128.pow(38) - 1;
const MAX_DATE: i32 = 106_751_991;
const MICROS_PER_DAY: i64 = 86_400_000_000;

fn order_key(bytes: &[u8]) -> OrderKey {
    OrderKey::new(bytes.to_vec()).unwrap()
}

fn sum(ctors: Vec<(&[u8], Vec<KeyType>)>) -> KeyType {
    let ctors = ctors
        .into_iter()
        .map(|(k, fields)| CtorType {
            order_key: order_key(k),
            fields,
        })
        .collect();
    KeyType::Sum(SumType::new(ctors).unwrap())
}

fn option(ty: KeyType) -> KeyType {
    sum(vec![(&[0x40], vec![]), (&[0x80], vec![ty])])
}

fn none() -> Value {
    Value::Sum {
        ctor: 0,
        fields: vec![],
    }
}

fn some(v: Value) -> Value {
    Value::Sum {
        ctor: 1,
        fields: vec![v],
    }
}

fn scalar(s: Scalar) -> KeyType {
    KeyType::Scalar(s)
}

fn asc(tys: Vec<KeyType>) -> KeySchema {
    KeySchema::new(tys.into_iter().map(|t| (t, Dir::Asc)).collect())
}

fn gen_order_keys(rng: &mut Rng, n: usize) -> Vec<OrderKey> {
    let mut keys = BTreeSet::new();
    while keys.len() < n {
        let len = 1 + rng.below(3);
        let bytes: Vec<u8> = (0..len)
            .map(|_| rng.pick(&[0x01, 0x02, 0x7F, 0x80, 0x81, 0xFF]))
            .collect();
        if let Some(k) = OrderKey::new(bytes) {
            keys.insert(k);
        }
    }
    let mut keys: Vec<OrderKey> = keys.into_iter().collect();
    while keys.len() > n {
        keys.remove(rng.below(keys.len()));
    }
    keys
}

fn gen_type(rng: &mut Rng, depth: usize, ordered: bool) -> KeyType {
    let kind = if depth == 0 { 0 } else { rng.below(8) };
    match kind {
        0..=3 => {
            if !ordered && rng.chance(5) {
                scalar(rng.pick(&UNORDERED))
            } else {
                scalar(rng.pick(&ORDERED))
            }
        }
        4 => KeyType::Record(
            (0..rng.below(3))
                .map(|_| gen_type(rng, depth - 1, ordered))
                .collect(),
        ),
        6 => KeyType::AnonRecord(gen_anon_record(rng, depth - 1, ordered)),
        7 => {
            let n = 1 + rng.below(4);
            let ctors = gen_names(rng, n)
                .into_iter()
                .map(|name| (name, gen_anon_record(rng, depth - 1, ordered)))
                .collect();
            KeyType::AnonSum(AnonSumType::new(ctors).unwrap())
        }
        _ => {
            let n = 1 + rng.below(4);
            let ctors = gen_order_keys(rng, n)
                .into_iter()
                .map(|order_key| CtorType {
                    order_key,
                    fields: (0..rng.below(3))
                        .map(|_| gen_type(rng, depth - 1, ordered))
                        .collect(),
                })
                .collect();
            KeyType::Sum(SumType::new(ctors).unwrap())
        }
    }
}

const NAMES: [&str; 5] = ["a", "b", "ab", "z", "\u{e9}"];

fn gen_names(rng: &mut Rng, n: usize) -> Vec<String> {
    let mut names: Vec<String> = NAMES.iter().map(ToString::to_string).collect();
    while names.len() > n {
        names.remove(rng.below(names.len()));
    }
    names.reverse();
    names
}

fn gen_anon_record(rng: &mut Rng, depth: usize, ordered: bool) -> AnonRecordType {
    let n = rng.below(3);
    let fields = gen_names(rng, n)
        .into_iter()
        .map(|name| (name, gen_type(rng, depth, ordered)))
        .collect();
    AnonRecordType::new(fields).unwrap()
}

fn gen_schema(rng: &mut Rng, ordered: bool) -> KeySchema {
    KeySchema::new(
        (0..=rng.below(3))
            .map(|_| {
                let dir = if rng.chance(2) { Dir::Asc } else { Dir::Desc };
                (gen_type(rng, 3, ordered), dir)
            })
            .collect(),
    )
}

fn gen_int(rng: &mut Rng, min: i128, max: i128) -> i128 {
    match rng.below(4) {
        0 => rng
            .pick(&[min, min + 1, -1, 0, 1, max - 1, max])
            .clamp(min, max),
        1 => (rng.below(7) as i128 - 3).clamp(min, max),
        _ => {
            let span = max.abs_diff(min) + 1;
            let offset = (u128::from(rng.next()) << 64 | u128::from(rng.next())) % span;
            min.checked_add_unsigned(offset).unwrap()
        }
    }
}

fn gen_f64(rng: &mut Rng) -> f64 {
    if rng.chance(4) {
        f64::from_bits(rng.next())
    } else {
        rng.pick(&[
            f64::NEG_INFINITY,
            f64::MIN,
            -1.0,
            -f64::MIN_POSITIVE,
            -5e-324,
            -0.0,
            0.0,
            5e-324,
            f64::MIN_POSITIVE,
            1.0,
            1.5,
            f64::MAX,
            f64::INFINITY,
            f64::NAN,
            -f64::NAN,
        ])
    }
}

fn gen_f32(rng: &mut Rng) -> f32 {
    if rng.chance(4) {
        f32::from_bits(rng.next() as u32)
    } else {
        rng.pick(&[
            f32::NEG_INFINITY,
            f32::MIN,
            -1.0,
            -f32::MIN_POSITIVE,
            -1e-45,
            -0.0,
            0.0,
            1e-45,
            f32::MIN_POSITIVE,
            1.0,
            f32::MAX,
            f32::INFINITY,
            f32::NAN,
        ])
    }
}

fn gen_text(rng: &mut Rng) -> String {
    (0..rng.below(4)).map(|_| rng.pick(&TEXT_CHARS)).collect()
}

fn gen_bytes(rng: &mut Rng) -> Vec<u8> {
    (0..rng.below(4)).map(|_| rng.pick(&BYTES)).collect()
}

fn gen_i64(rng: &mut Rng) -> i64 {
    gen_int(rng, i64::MIN.into(), i64::MAX.into()) as i64
}

fn gen_i32(rng: &mut Rng) -> i32 {
    gen_int(rng, i32::MIN.into(), i32::MAX.into()) as i32
}

fn gen_scalar(rng: &mut Rng, s: Scalar) -> Value {
    match s {
        Scalar::I8 => Value::I8(gen_int(rng, i8::MIN.into(), i8::MAX.into()) as i8),
        Scalar::I16 => Value::I16(gen_int(rng, i16::MIN.into(), i16::MAX.into()) as i16),
        Scalar::I32 => Value::I32(gen_i32(rng)),
        Scalar::I64 => Value::I64(gen_i64(rng)),
        Scalar::F32 => Value::F32(F32::new(gen_f32(rng))),
        Scalar::F64 => Value::F64(F64::new(gen_f64(rng))),
        Scalar::Decimal(_) => Value::Decimal(gen_int(rng, -MAX_DECIMAL, MAX_DECIMAL)),
        Scalar::Text => Value::Text(gen_text(rng)),
        Scalar::Bytes => Value::Bytes(gen_bytes(rng)),
        Scalar::SortKey => Value::SortKey(gen_bytes(rng)),
        Scalar::Instant => Value::Instant(gen_i64(rng)),
        Scalar::Date => Value::Date(gen_int(rng, (-MAX_DATE).into(), MAX_DATE.into()) as i32),
        Scalar::Time => Value::Time(gen_int(rng, 0, (MICROS_PER_DAY - 1).into()) as i64),
        Scalar::DateTime => Value::DateTime(gen_i64(rng)),
        Scalar::Duration => Value::Duration(gen_i64(rng)),
        Scalar::ZonedDateTime => Value::ZonedDateTime {
            local: gen_i64(rng),
            zone: rng.pick(&["UTC", "Europe/Lisbon", "\0"]).to_string(),
        },
        Scalar::Period => Value::Period {
            months: gen_i32(rng),
            days: gen_i32(rng),
        },
    }
}

fn gen_value(rng: &mut Rng, ty: &KeyType) -> Value {
    match ty {
        KeyType::Scalar(s) => gen_scalar(rng, *s),
        KeyType::Record(tys) => Value::Record(tys.iter().map(|t| gen_value(rng, t)).collect()),
        KeyType::AnonRecord(rec) => Value::Record(
            rec.fields()
                .iter()
                .map(|(_, t)| gen_value(rng, t))
                .collect(),
        ),
        KeyType::AnonSum(sum) => {
            let ctor = rng.below(sum.ctors().len());
            Value::Sum {
                ctor: ctor as u32,
                fields: sum.ctors()[ctor]
                    .1
                    .fields()
                    .iter()
                    .map(|(_, t)| gen_value(rng, t))
                    .collect(),
            }
        }
        KeyType::Sum(sum) => {
            let ctor = rng.below(sum.ctors().len());
            Value::Sum {
                ctor: ctor as u32,
                fields: sum.ctors()[ctor]
                    .fields
                    .iter()
                    .map(|t| gen_value(rng, t))
                    .collect(),
            }
        }
    }
}

fn gen_key(rng: &mut Rng, schema: &KeySchema) -> Vec<Value> {
    schema
        .components()
        .iter()
        .map(|(ty, _)| gen_value(rng, ty))
        .collect()
}

fn gen_nearby_key(rng: &mut Rng, schema: &KeySchema, near: &[Value]) -> Vec<Value> {
    let mut key = near.to_vec();
    if rng.chance(3) {
        return gen_key(rng, schema);
    }
    let i = rng.below(key.len());
    key[i] = gen_value(rng, &schema.components()[i].0);
    key
}

fn cmp_float(x: f64, y: f64) -> Ordering {
    match (x.is_nan(), y.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => x.partial_cmp(&y).unwrap(),
    }
}

fn cmp_values(xs: &[Value], ys: &[Value]) -> Ordering {
    assert_eq!(xs.len(), ys.len());
    xs.iter()
        .zip(ys)
        .map(|(x, y)| cmp_value(x, y))
        .find(|o| o.is_ne())
        .unwrap_or(Ordering::Equal)
}

fn cmp_value(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::I8(x), Value::I8(y)) => x.cmp(y),
        (Value::I16(x), Value::I16(y)) => x.cmp(y),
        (Value::I32(x), Value::I32(y)) | (Value::Date(x), Value::Date(y)) => x.cmp(y),
        (Value::I64(x), Value::I64(y))
        | (Value::Instant(x), Value::Instant(y))
        | (Value::Time(x), Value::Time(y))
        | (Value::DateTime(x), Value::DateTime(y))
        | (Value::Duration(x), Value::Duration(y)) => x.cmp(y),
        (Value::F32(x), Value::F32(y)) => cmp_float(x.get().into(), y.get().into()),
        (Value::F64(x), Value::F64(y)) => cmp_float(x.get(), y.get()),
        (Value::Decimal(x), Value::Decimal(y)) => x.cmp(y),
        (Value::Text(x), Value::Text(y)) => x.chars().cmp(y.chars()),
        (Value::Bytes(x), Value::Bytes(y)) | (Value::SortKey(x), Value::SortKey(y)) => x.cmp(y),
        (Value::Record(xs), Value::Record(ys)) => cmp_values(xs, ys),
        (
            Value::Sum {
                ctor: c,
                fields: xs,
            },
            Value::Sum {
                ctor: d,
                fields: ys,
            },
        ) => c.cmp(d).then_with(|| cmp_values(xs, ys)),
        _ => panic!("no order between {a:?} and {b:?}"),
    }
}

fn cmp_keys(schema: &KeySchema, xs: &[Value], ys: &[Value]) -> Ordering {
    schema
        .components()
        .iter()
        .zip(xs.iter().zip(ys))
        .map(|((_, dir), (x, y))| match dir {
            Dir::Asc => cmp_value(x, y),
            Dir::Desc => cmp_value(x, y).reverse(),
        })
        .find(|o| o.is_ne())
        .unwrap_or(Ordering::Equal)
}

#[test]
fn generated_keys_round_trip() {
    let mut rng = Rng(1);
    for _ in 0..5000 {
        let schema = gen_schema(&mut rng, false);
        let key = gen_key(&mut rng, &schema);
        let bytes = schema.encode(&key);
        assert_eq!(
            schema.decode(&bytes),
            Ok(key.clone()),
            "{schema:?} {bytes:x?}"
        );
        let other = gen_nearby_key(&mut rng, &schema, &key);
        assert_eq!(
            key == other,
            bytes == schema.encode(&other),
            "{key:?} {other:?}"
        );
    }
}

#[test]
fn generated_keys_preserve_order() {
    let mut rng = Rng(2);
    for _ in 0..20000 {
        let schema = gen_schema(&mut rng, true);
        let a = gen_key(&mut rng, &schema);
        let b = gen_nearby_key(&mut rng, &schema, &a);
        assert_eq!(
            schema.encode(&a).cmp(&schema.encode(&b)),
            cmp_keys(&schema, &a, &b),
            "{schema:?}\n{a:?}\n{b:?}"
        );
    }
}

#[test]
fn decoding_accepts_only_encodings() {
    let mut rng = Rng(3);
    for _ in 0..20000 {
        let schema = gen_schema(&mut rng, false);
        let mut bytes = schema.encode(&gen_key(&mut rng, &schema));
        match rng.below(4) {
            0 if !bytes.is_empty() => {
                let i = rng.below(bytes.len());
                bytes[i] = rng.next() as u8;
            }
            1 if !bytes.is_empty() => bytes.truncate(rng.below(bytes.len())),
            2 => {
                let i = rng.below(bytes.len() + 1);
                bytes.insert(i, rng.pick(&BYTES));
            }
            _ => bytes = (0..rng.below(12)).map(|_| rng.next() as u8).collect(),
        }
        if let Ok(key) = schema.decode(&bytes) {
            assert_eq!(schema.encode(&key), bytes, "{schema:?} {key:?}");
        }
    }
}

#[test]
fn generated_entries_order_and_round_trip() {
    let ctors = [0, 1, 239, 240, 255, 256, 65535, 65536, u32::MAX];
    let mut rng = Rng(4);
    for _ in 0..10000 {
        let schema = gen_schema(&mut rng, true);
        let mut entry = || Entry {
            key: gen_key(&mut rng, &schema),
            ctor: CtorId(rng.pick(&ctors)),
            tid: Tid::new(
                rng.pick(&[0, 1, Tid::PAGE_LIMIT - 1]),
                rng.pick(&[0, 1, u16::MAX]),
            )
            .unwrap(),
        };
        let (a, b) = (entry(), entry());
        let (ea, eb) = (schema.encode_entry(&a), schema.encode_entry(&b));
        assert_eq!(schema.decode_entry(&ea), Ok(a.clone()));
        let expected = cmp_keys(&schema, &a.key, &b.key)
            .then(a.ctor.cmp(&b.ctor))
            .then(a.tid.cmp(&b.tid));
        assert_eq!(ea.cmp(&eb), expected, "{a:?} {b:?}");

        let mut seek = schema.encode(&a.key);
        ovarint::encode(a.ctor.0, &mut seek);
        assert!(ea.starts_with(&seek));
        let same_range = a.key == b.key && a.ctor == b.ctor;
        assert_eq!(eb.starts_with(&seek), same_range, "{a:?} {b:?}");
    }
}

fn ovarint_bytes(n: u32) -> Vec<u8> {
    let mut out = Vec::new();
    ovarint::encode(n, &mut out);
    out
}

fn ovarint_decode(bytes: &[u8]) -> Result<u32, DecodeError> {
    let mut r = Reader::new(bytes);
    let n = ovarint::decode(&mut r)?;
    r.finish()?;
    Ok(n)
}

#[test]
fn ovarint_bytes_and_order() {
    let cases: [(u32, &[u8]); 10] = [
        (0, &[0x00]),
        (1, &[0x01]),
        (239, &[0xEF]),
        (240, &[0xF0, 0xF0]),
        (255, &[0xF0, 0xFF]),
        (256, &[0xF1, 0x01, 0x00]),
        (65535, &[0xF1, 0xFF, 0xFF]),
        (65536, &[0xF2, 0x01, 0x00, 0x00]),
        (1 << 24, &[0xF3, 0x01, 0x00, 0x00, 0x00]),
        (u32::MAX, &[0xF3, 0xFF, 0xFF, 0xFF, 0xFF]),
    ];
    for (n, bytes) in cases {
        assert_eq!(ovarint_bytes(n), bytes);
        assert_eq!(ovarint_decode(bytes), Ok(n));
    }
    for w in cases.windows(2) {
        assert!(ovarint_bytes(w[0].0) < ovarint_bytes(w[1].0));
    }
    let mut rng = Rng(5);
    for _ in 0..10000 {
        let (a, b) = (
            rng.next() as u32 >> rng.below(32),
            rng.next() as u32 >> rng.below(32),
        );
        assert_eq!(ovarint_bytes(a).cmp(&ovarint_bytes(b)), a.cmp(&b));
        assert_eq!(ovarint_decode(&ovarint_bytes(a)), Ok(a));
    }
}

#[test]
fn ovarint_refuses_other_forms() {
    let bad: [&[u8]; 9] = [
        &[0xF0, 0xEF],
        &[0xF0, 0x05],
        &[0xF1, 0x00, 0xFF],
        &[0xF2, 0x00, 0xFF, 0xFF],
        &[0xF3, 0x00, 0xFF, 0xFF, 0xFF],
        &[0xF4, 0x01, 0x00, 0x00, 0x00, 0x00],
        &[0xFF],
        &[0xF1, 0x01],
        &[],
    ];
    for bytes in bad {
        assert!(ovarint_decode(bytes).is_err(), "{bytes:x?}");
    }
}

#[test]
fn integers_flip_the_sign_bit() {
    let i8s = asc(vec![scalar(Scalar::I8)]);
    assert_eq!(i8s.encode(&[Value::I8(i8::MIN)]), [0x00]);
    assert_eq!(i8s.encode(&[Value::I8(-1)]), [0x7F]);
    assert_eq!(i8s.encode(&[Value::I8(0)]), [0x80]);
    assert_eq!(i8s.encode(&[Value::I8(i8::MAX)]), [0xFF]);
    let i64s = asc(vec![scalar(Scalar::I64)]);
    assert_eq!(i64s.encode(&[Value::I64(3)]), [0x80, 0, 0, 0, 0, 0, 0, 3]);
    assert_eq!(i64s.encode(&[Value::I64(i64::MIN)]), [0; 8]);
    let dates = asc(vec![scalar(Scalar::Date)]);
    assert_eq!(dates.encode(&[Value::Date(-1)]), [0x7F, 0xFF, 0xFF, 0xFF]);
}

#[test]
fn decimals_are_sixteen_bytes_whatever_the_scale() {
    for scale in [0, 2, 38] {
        let schema = asc(vec![scalar(Scalar::Decimal(scale))]);
        let mut expected = vec![0x80];
        expected.extend([0; 14]);
        expected.push(1);
        assert_eq!(schema.encode(&[Value::Decimal(1)]), expected);
        let ordered: Vec<Vec<u8>> = [-MAX_DECIMAL, -1, 0, 1, MAX_DECIMAL]
            .iter()
            .map(|&n| schema.encode(&[Value::Decimal(n)]))
            .collect();
        assert!(ordered.is_sorted());
    }
}

#[test]
fn text_escapes_zero_and_ends_with_zero_one() {
    let schema = asc(vec![scalar(Scalar::Text)]);
    assert_eq!(schema.encode(&[Value::Text(String::new())]), [0x00, 0x01]);
    assert_eq!(
        schema.encode(&[Value::Text("foo\0bar".into())]),
        [0x66, 0x6F, 0x6F, 0x00, 0xFF, 0x62, 0x61, 0x72, 0x00, 0x01]
    );
    let bytes = asc(vec![scalar(Scalar::Bytes)]);
    assert_eq!(
        bytes.encode(&[Value::Bytes(vec![0x00])]),
        [0x00, 0xFF, 0x00, 0x01]
    );
}

#[test]
fn text_order_is_code_point_order() {
    let schema = asc(vec![scalar(Scalar::Text)]);
    let ascending = [
        "",
        "\0",
        "\0\0",
        "\0\u{1}",
        "\u{1}",
        "a",
        "a\0",
        "a\0\0",
        "a\u{1}",
        "ab",
        "\u{7F}",
        "\u{80}",
        "\u{FFFF}",
        "\u{10000}",
        "\u{10FFFF}",
    ];
    let encoded: Vec<Vec<u8>> = ascending
        .iter()
        .map(|t| schema.encode(&[Value::Text((*t).into())]))
        .collect();
    assert!(encoded.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn text_is_prefix_free_before_another_component() {
    let schema = asc(vec![scalar(Scalar::Text), scalar(Scalar::I16)]);
    let short = schema.encode(&[Value::Text("a".into()), Value::I16(i16::MAX)]);
    let long = schema.encode(&[Value::Text("a\0".into()), Value::I16(i16::MIN)]);
    assert!(short < long);
}

#[test]
fn descending_components_invert_the_order() {
    let schema = KeySchema::new(vec![
        (scalar(Scalar::Text), Dir::Desc),
        (scalar(Scalar::I64), Dir::Asc),
    ]);
    let ascending = ["", "\0", "a", "a\0", "ab", "b"];
    let encoded: Vec<Vec<u8>> = ascending
        .iter()
        .map(|t| schema.encode(&[Value::Text((*t).into()), Value::I64(0)]))
        .collect();
    assert!(encoded.windows(2).all(|w| w[0] > w[1]));
    for (t, bytes) in ascending.iter().zip(&encoded) {
        assert_eq!(
            schema.decode(bytes),
            Ok(vec![Value::Text((*t).into()), Value::I64(0)])
        );
    }
    let desc_i8 = KeySchema::new(vec![(scalar(Scalar::I8), Dir::Desc)]);
    assert_eq!(desc_i8.encode(&[Value::I8(0)]), [0x7F]);
}

#[test]
fn floats_are_canonical_and_ordered() {
    assert_eq!(F64::new(-0.0), F64::new(0.0));
    assert_eq!(F64::new(f64::NAN), F64::new(-f64::NAN));
    assert_eq!(
        F64::new(f64::from_bits(0x7FF0_0000_0000_0001)),
        F64::new(f64::NAN)
    );
    assert_eq!(F32::new(-0.0), F32::new(0.0));
    assert_eq!(F32::new(f32::from_bits(0xFF80_0001)), F32::new(f32::NAN));

    let f64s = asc(vec![scalar(Scalar::F64)]);
    let encode64 = |x: f64| f64s.encode(&[Value::F64(F64::new(x))]);
    assert_eq!(encode64(-0.0), encode64(0.0));
    assert_eq!(encode64(0.0), [0x80, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(encode64(f64::NAN), [0xFF, 0xF8, 0, 0, 0, 0, 0, 0]);
    let ascending64 = [
        f64::NEG_INFINITY,
        f64::MIN,
        -1.0,
        -f64::MIN_POSITIVE,
        -5e-324,
        0.0,
        5e-324,
        f64::MIN_POSITIVE,
        1.0,
        f64::MAX,
        f64::INFINITY,
        f64::NAN,
    ];
    let encoded: Vec<Vec<u8>> = ascending64.iter().map(|&x| encode64(x)).collect();
    assert!(encoded.windows(2).all(|w| w[0] < w[1]));

    let f32s = asc(vec![scalar(Scalar::F32)]);
    let encode32 = |x: f32| f32s.encode(&[Value::F32(F32::new(x))]);
    assert_eq!(encode32(-0.0), encode32(0.0));
    assert_eq!(encode32(f32::NAN), [0xFF, 0xC0, 0, 0]);
    let ascending32 = [
        f32::NEG_INFINITY,
        f32::MIN,
        -1.0,
        -1e-45,
        0.0,
        1e-45,
        1.0,
        f32::MAX,
        f32::INFINITY,
        f32::NAN,
    ];
    let encoded: Vec<Vec<u8>> = ascending32.iter().map(|&x| encode32(x)).collect();
    assert!(encoded.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn sum_values_encode_order_key_then_payload() {
    let status = sum(vec![
        (&[0x40], vec![]),
        (&[0x80], vec![scalar(Scalar::Instant)]),
        (&[0xC0], vec![scalar(Scalar::Instant)]),
    ]);
    let schema = asc(vec![status]);
    let t: i64 = 1_700_000_000_000_000;
    let sent = Value::Sum {
        ctor: 1,
        fields: vec![Value::Instant(t)],
    };
    let mut expected = vec![0x80, 0x00];
    expected.extend((t as u64 ^ 1 << 63).to_be_bytes());
    assert_eq!(schema.encode(std::slice::from_ref(&sent)), expected);
    let draft = Value::Sum {
        ctor: 0,
        fields: vec![],
    };
    assert_eq!(schema.encode(&[draft]), [0x40, 0x00]);
}

#[test]
fn sums_order_by_declaration_through_order_keys() {
    let ty = sum(vec![
        (&[0x80], vec![scalar(Scalar::I8)]),
        (&[0x80, 0x01, 0x80], vec![]),
        (&[0x80, 0x80], vec![]),
        (&[0x81], vec![scalar(Scalar::Text)]),
    ]);
    let schema = asc(vec![ty]);
    let ascending = [
        Value::Sum {
            ctor: 0,
            fields: vec![Value::I8(i8::MIN)],
        },
        Value::Sum {
            ctor: 0,
            fields: vec![Value::I8(i8::MAX)],
        },
        Value::Sum {
            ctor: 1,
            fields: vec![],
        },
        Value::Sum {
            ctor: 2,
            fields: vec![],
        },
        Value::Sum {
            ctor: 3,
            fields: vec![Value::Text(String::new())],
        },
    ];
    let encoded: Vec<Vec<u8>> = ascending
        .iter()
        .map(|v| schema.encode(std::slice::from_ref(v)))
        .collect();
    assert!(encoded.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn nested_options_order() {
    let schema = asc(vec![option(option(scalar(Scalar::I64)))]);
    let ascending = [
        none(),
        some(none()),
        some(some(Value::I64(i64::MIN))),
        some(some(Value::I64(0))),
        some(some(Value::I64(i64::MAX))),
    ];
    let encoded: Vec<Vec<u8>> = ascending
        .iter()
        .map(|v| schema.encode(std::slice::from_ref(v)))
        .collect();
    assert!(encoded.windows(2).all(|w| w[0] < w[1]));
    for (v, bytes) in ascending.iter().zip(&encoded) {
        assert_eq!(schema.decode(bytes), Ok(vec![v.clone()]));
    }
}

#[test]
fn records_concatenate_fields() {
    let point = KeyType::Record(vec![scalar(Scalar::I8), scalar(Scalar::Text)]);
    let schema = asc(vec![point]);
    let v = Value::Record(vec![Value::I8(1), Value::Text("x".into())]);
    assert_eq!(
        schema.encode(std::slice::from_ref(&v)),
        [0x81, 0x78, 0x00, 0x01]
    );
    let empty = asc(vec![KeyType::Record(vec![])]);
    assert_eq!(empty.encode(&[Value::Record(vec![])]), Vec::<u8>::new());
}

#[test]
fn unordered_types_encode_their_parts() {
    let schema = asc(vec![scalar(Scalar::ZonedDateTime), scalar(Scalar::Period)]);
    let key = [
        Value::ZonedDateTime {
            local: 0,
            zone: "UTC".into(),
        },
        Value::Period {
            months: 1,
            days: -1,
        },
    ];
    let bytes = schema.encode(&key);
    assert_eq!(
        bytes,
        [
            0x80, 0, 0, 0, 0, 0, 0, 0, b'U', b'T', b'C', 0x00, 0x01, 0x80, 0, 0, 1, 0x7F, 0xFF,
            0xFF, 0xFF
        ]
    );
    assert_eq!(schema.decode(&bytes), Ok(key.to_vec()));
}

fn decode_one(s: Scalar, bytes: &[u8]) -> Result<Vec<Value>, DecodeError> {
    asc(vec![scalar(s)]).decode(bytes)
}

#[test]
fn decoding_refuses_malformed_scalars() {
    use DecodeError::*;
    assert_eq!(decode_one(Scalar::I64, &[0x80; 7]), Err(UnexpectedEnd));
    assert_eq!(decode_one(Scalar::I8, &[0x80, 0x00]), Err(TrailingBytes));
    assert_eq!(decode_one(Scalar::Text, &[0x61]), Err(UnexpectedEnd));
    assert_eq!(decode_one(Scalar::Text, &[0x61, 0x00]), Err(UnexpectedEnd));
    assert_eq!(
        decode_one(Scalar::Text, &[0x61, 0x00, 0x02]),
        Err(BadEscape)
    );
    assert_eq!(
        decode_one(Scalar::Text, &[0x61, 0x00, 0x00]),
        Err(BadEscape)
    );
    assert_eq!(
        decode_one(Scalar::Text, &[0xFF, 0x00, 0x01]),
        Err(InvalidUtf8)
    );
    assert_eq!(
        decode_one(Scalar::Text, &[0xED, 0xA0, 0x80, 0x00, 0x01]),
        Err(InvalidUtf8)
    );
    assert_eq!(
        decode_one(Scalar::Bytes, &[0xFF, 0x00, 0x01]),
        Ok(vec![Value::Bytes(vec![0xFF])])
    );
}

#[test]
fn decoding_refuses_non_canonical_floats() {
    let encoded = |bits: u64| {
        let order = if bits >> 63 == 1 {
            !bits
        } else {
            bits ^ 1 << 63
        };
        order.to_be_bytes()
    };
    let refused = [
        (-0.0_f64).to_bits(),
        0x7FF8_0000_0000_0001,
        0x7FF0_0000_0000_0001,
        0xFFF8_0000_0000_0000,
    ];
    for bits in refused {
        assert_eq!(
            decode_one(Scalar::F64, &encoded(bits)),
            Err(DecodeError::NonCanonicalFloat),
            "{bits:x}"
        );
    }
    assert_eq!(
        decode_one(Scalar::F64, &encoded(F64::NAN_BITS)),
        Ok(vec![Value::F64(F64::new(f64::NAN))])
    );
    assert_eq!(
        decode_one(Scalar::F32, &[0x7F, 0xFF, 0xFF, 0xFF]),
        Err(DecodeError::NonCanonicalFloat)
    );
}

#[test]
fn decoding_refuses_out_of_range_values() {
    let decimal = |n: i128| {
        let mut b = n.to_be_bytes();
        b[0] ^= 0x80;
        decode_one(Scalar::Decimal(0), &b)
    };
    assert_eq!(decimal(MAX_DECIMAL), Ok(vec![Value::Decimal(MAX_DECIMAL)]));
    assert_eq!(
        decimal(-MAX_DECIMAL),
        Ok(vec![Value::Decimal(-MAX_DECIMAL)])
    );
    assert_eq!(
        decimal(MAX_DECIMAL + 1),
        Err(DecodeError::DecimalOutOfRange)
    );
    assert_eq!(
        decimal(-MAX_DECIMAL - 1),
        Err(DecodeError::DecimalOutOfRange)
    );
    assert_eq!(decimal(i128::MIN), Err(DecodeError::DecimalOutOfRange));

    let time = |t: i64| decode_one(Scalar::Time, &(t as u64 ^ 1 << 63).to_be_bytes());
    assert_eq!(
        time(MICROS_PER_DAY - 1),
        Ok(vec![Value::Time(MICROS_PER_DAY - 1)])
    );
    assert_eq!(time(MICROS_PER_DAY), Err(DecodeError::TimeOutOfRange));
    assert_eq!(time(-1), Err(DecodeError::TimeOutOfRange));

    let date = |d: i32| decode_one(Scalar::Date, &(d as u32 ^ 1 << 31).to_be_bytes());
    assert_eq!(date(MAX_DATE), Ok(vec![Value::Date(MAX_DATE)]));
    assert_eq!(date(-MAX_DATE), Ok(vec![Value::Date(-MAX_DATE)]));
    assert_eq!(date(MAX_DATE + 1), Err(DecodeError::DateOutOfRange));
    assert_eq!(date(-MAX_DATE - 1), Err(DecodeError::DateOutOfRange));
}

#[test]
fn decoding_refuses_unknown_order_keys() {
    let schema = asc(vec![option(scalar(Scalar::I8))]);
    assert_eq!(
        schema.decode(&[0x60, 0x00]),
        Err(DecodeError::UnknownOrderKey)
    );
    assert_eq!(schema.decode(&[0x40]), Err(DecodeError::UnexpectedEnd));
    assert_eq!(
        schema.decode(&[0x80, 0x00]),
        Err(DecodeError::UnexpectedEnd)
    );
    assert_eq!(schema.decode(&[0x40, 0x00]), Ok(vec![none()]));
}

#[test]
fn decoding_refuses_malformed_entries() {
    let schema = asc(vec![scalar(Scalar::I8)]);
    let entry = Entry {
        key: vec![Value::I8(7)],
        ctor: CtorId(300),
        tid: Tid::new(12, 3).unwrap(),
    };
    let bytes = schema.encode_entry(&entry);
    assert_eq!(bytes, [0x87, 0xF1, 0x01, 0x2C, 0, 0, 0, 0, 0, 12, 0, 3]);
    assert_eq!(schema.decode_entry(&bytes), Ok(entry));
    assert_eq!(
        schema.decode_entry(&bytes[..bytes.len() - 1]),
        Err(DecodeError::UnexpectedEnd)
    );
    let mut long = bytes;
    long.push(0);
    assert_eq!(schema.decode_entry(&long), Err(DecodeError::TrailingBytes));
    assert_eq!(
        schema.decode_entry(&[0x87, 0xF0, 0x05, 0, 0, 0, 0, 0, 12, 0, 3]),
        Err(DecodeError::BadVarint)
    );
}

#[test]
fn tid_pages_fit_in_48_bits() {
    assert_eq!(Tid::new(Tid::PAGE_LIMIT, 0), None);
    assert_eq!(
        Tid::new(Tid::PAGE_LIMIT - 1, u16::MAX).map(|t| (t.page(), t.slot())),
        Some((Tid::PAGE_LIMIT - 1, u16::MAX))
    );
}

#[test]
fn order_keys_and_sums_are_validated() {
    assert_eq!(OrderKey::new(vec![]), None);
    assert_eq!(OrderKey::new(vec![0x00]), None);
    assert_eq!(OrderKey::new(vec![0x80, 0x00, 0x80]), None);
    assert_eq!(OrderKey::new(vec![0x80, 0x01]), None);
    assert!(OrderKey::new(vec![0x01, 0x80]).is_some());
    let ctor = |k: &[u8]| CtorType {
        order_key: order_key(k),
        fields: vec![],
    };
    assert!(SumType::new(vec![ctor(&[0x80]), ctor(&[0x40])]).is_none());
    assert!(SumType::new(vec![ctor(&[0x80]), ctor(&[0x80])]).is_none());
    assert!(SumType::new(vec![ctor(&[0x80]), ctor(&[0x80, 0x80])]).is_some());
}

fn iota(n: u8) -> Vec<u8> {
    (0..n).collect()
}

#[test]
fn hash64_is_xxh64_with_seed_zero() {
    assert_eq!(hash64(b""), 0xEF46_DB37_51D8_E999);
    assert_eq!(hash64(b"a"), 0xD24E_C4F1_A98C_6E5B);
    assert_eq!(hash64(b"abc"), 0x44BC_2CF5_AD77_0999);
    assert_eq!(hash64(&iota(7)), 0x14CC_643F_630C_72D2);
    assert_eq!(hash64(&iota(12)), 0x424A_F23F_1F08_DCA5);
    assert_eq!(hash64(&iota(32)), 0xCBF5_9C51_16FF_32B4);
    assert_eq!(hash64(&iota(33)), 0x0C53_5D1A_CAFB_8EAD);
    assert_eq!(hash64(&iota(47)), 0x0D98_83A0_3E7B_FBB8);
    assert_eq!(hash64(&iota(100)), 0x6AC1_E580_3216_6597);
    assert_eq!(hash32(b"abc"), 0x44BC_2CF5);
}

#[test]
fn equal_values_hash_equally() {
    let schema = asc(vec![scalar(Scalar::F64)]);
    let hash = |x: f64| hash64(&schema.encode(&[Value::F64(F64::new(x))]));
    assert_eq!(hash(-0.0), hash(0.0));
    assert_eq!(hash(f64::NAN), hash(f64::from_bits(0xFFF0_0000_0000_0001)));
}

fn anon(fields: Vec<(&str, KeyType)>) -> AnonRecordType {
    AnonRecordType::new(
        fields
            .into_iter()
            .map(|(n, t)| (n.to_string(), t))
            .collect(),
    )
    .unwrap()
}

#[test]
fn anonymous_records_encode_fields_in_name_order() {
    let ty = anon(vec![("y", scalar(Scalar::I8)), ("x", scalar(Scalar::Text))]);
    let names: Vec<&str> = ty.fields().iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["x", "y"]);
    let respelled = anon(vec![("x", scalar(Scalar::Text)), ("y", scalar(Scalar::I8))]);
    assert_eq!(ty, respelled);
    let schema = asc(vec![KeyType::AnonRecord(ty)]);
    let v = Value::Record(vec![Value::Text("x".into()), Value::I8(1)]);
    assert_eq!(
        schema.encode(std::slice::from_ref(&v)),
        [0x78, 0x00, 0x01, 0x81]
    );
    assert_eq!(
        AnonRecordType::new(vec![
            ("x".into(), scalar(Scalar::I8)),
            ("x".into(), scalar(Scalar::I16)),
        ]),
        None
    );
}

#[test]
fn anonymous_sums_encode_the_name_order_index_then_fields() {
    let ctors = |order: [usize; 3]| {
        let all = [
            ("Some", anon(vec![("v", scalar(Scalar::I8))])),
            ("None", anon(vec![])),
            (
                "Many",
                anon(vec![("n", scalar(Scalar::I16)), ("m", scalar(Scalar::I8))]),
            ),
        ];
        let ctors = order
            .iter()
            .map(|&i| (all[i].0.to_string(), all[i].1.clone()))
            .collect();
        AnonSumType::new(ctors).unwrap()
    };
    let ty = ctors([0, 1, 2]);
    assert_eq!(ty, ctors([2, 0, 1]));
    let names: Vec<&str> = ty.ctors().iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["Many", "None", "Some"]);
    let schema = asc(vec![KeyType::AnonSum(ty)]);
    let many = Value::Sum {
        ctor: 0,
        fields: vec![Value::I8(-1), Value::I16(2)],
    };
    assert_eq!(
        schema.encode(std::slice::from_ref(&many)),
        [0x00, 0x7F, 0x80, 0x02]
    );
    let some = Value::Sum {
        ctor: 2,
        fields: vec![Value::I8(0)],
    };
    assert_eq!(schema.encode(std::slice::from_ref(&some)), [0x02, 0x80]);
    assert_eq!(schema.decode(&[0x02, 0x80]), Ok(vec![some]));
    assert_eq!(schema.decode(&[0x03]), Err(DecodeError::UnknownConstructor));
    assert_eq!(schema.decode(&[0xF0, 0x01]), Err(DecodeError::BadVarint));
    let duplicate = vec![
        ("A".to_string(), anon(vec![])),
        ("A".to_string(), anon(vec![])),
    ];
    assert_eq!(AnonSumType::new(duplicate), None);
}

#[test]
fn descending_anonymous_sums_invert_the_index() {
    let ty =
        AnonSumType::new(vec![("A".into(), anon(vec![])), ("B".into(), anon(vec![]))]).unwrap();
    let schema = KeySchema::new(vec![(KeyType::AnonSum(ty), Dir::Desc)]);
    let b = Value::Sum {
        ctor: 1,
        fields: vec![],
    };
    assert_eq!(schema.encode(std::slice::from_ref(&b)), [0xFE]);
    assert_eq!(schema.decode(&[0xFE]), Ok(vec![b]));
}

fn gen_valid_order_key(rng: &mut Rng) -> OrderKey {
    loop {
        let len = 1 + rng.below(4);
        let bytes: Vec<u8> = (0..len)
            .map(|_| rng.pick(&[0x00, 0x01, 0x02, 0x03, 0x7F, 0x80, 0xFE, 0xFF]))
            .collect();
        if let Some(k) = OrderKey::new(bytes) {
            return k;
        }
    }
}

fn assert_valid(k: &OrderKey) {
    assert_eq!(OrderKey::new(k.as_bytes().to_vec()).as_ref(), Some(k));
}

#[test]
fn order_keys_fit_between_below_and_above_any_others() {
    let mut rng = Rng(5);
    for _ in 0..20000 {
        let a = gen_valid_order_key(&mut rng);
        let b = gen_valid_order_key(&mut rng);
        let below = OrderKey::between(None, Some(&a));
        assert_valid(&below);
        assert!(below < a, "{below:x?} {a:x?}");
        let above = OrderKey::between(Some(&a), None);
        assert_valid(&above);
        assert!(above > a, "{above:x?} {a:x?}");
        if a == b {
            continue;
        }
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        let mid = OrderKey::between(Some(&lo), Some(&hi));
        assert_valid(&mid);
        assert!(lo < mid && mid < hi, "{lo:x?} {mid:x?} {hi:x?}");
    }
}

#[test]
fn order_keys_between_neighbours_never_run_out() {
    let mut rng = Rng(6);
    let mut keys = OrderKey::spread(3);
    for _ in 0..3000 {
        let i = rng.below(keys.len() + 1);
        let lo = i.checked_sub(1).map(|j| &keys[j]);
        let k = OrderKey::between(lo, keys.get(i));
        assert_valid(&k);
        keys.insert(i, k);
    }
    assert!(keys.windows(2).all(|w| w[0] < w[1]));
    let mut tight = [order_key(&[0x80]), order_key(&[0x80, 0x02])];
    for _ in 0..200 {
        let k = OrderKey::between(Some(&tight[0]), Some(&tight[1]));
        assert_valid(&k);
        tight[1] = k;
    }
    assert!(tight[0] < tight[1]);
}

#[test]
fn spread_order_keys_split_the_room_evenly() {
    assert_eq!(
        OrderKey::spread(3),
        [order_key(&[0x40]), order_key(&[0x80]), order_key(&[0xC0])]
    );
    assert_eq!(OrderKey::spread(1), [order_key(&[0x80])]);
    assert_eq!(OrderKey::spread(0), []);
    for n in [2, 7, 255, 256, 1000] {
        let keys = OrderKey::spread(n);
        assert_eq!(keys.len(), n);
        keys.iter().for_each(assert_valid);
        assert!(keys.windows(2).all(|w| w[0] < w[1]), "{n}");
    }
}

#[test]
fn order_keys_encode_in_key_order_with_their_terminator() {
    let mut rng = Rng(7);
    for _ in 0..5000 {
        let a = gen_valid_order_key(&mut rng);
        let b = gen_valid_order_key(&mut rng);
        let enc = |k: &OrderKey| [k.as_bytes(), &[0x00]].concat();
        assert_eq!(enc(&a).cmp(&enc(&b)), a.cmp(&b), "{a:x?} {b:x?}");
    }
}

#[test]
fn inserting_and_moving_a_constructor_keeps_the_other_keys() {
    let keys = OrderKey::spread(4);
    let ctor = |k: &OrderKey| CtorType {
        order_key: k.clone(),
        fields: vec![],
    };
    let added = OrderKey::between(Some(&keys[1]), Some(&keys[2]));
    let mut with_added: Vec<CtorType> = keys.iter().map(ctor).collect();
    with_added.insert(2, ctor(&added));
    assert!(SumType::new(with_added).is_some());

    let moved = OrderKey::between(None, Some(&keys[0]));
    let mut reordered = vec![ctor(&moved)];
    reordered.extend([&keys[0], &keys[1], &keys[2]].map(ctor));
    let sum = SumType::new(reordered).unwrap();
    let unchanged: Vec<&OrderKey> = sum.ctors()[1..].iter().map(|c| &c.order_key).collect();
    assert_eq!(unchanged, [&keys[0], &keys[1], &keys[2]]);
}
