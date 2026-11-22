use crate::key::{CtorId, F32, F64, Scalar, Value};

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

const SCALARS: [Scalar; 17] = [
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
    Scalar::ZonedDateTime,
    Scalar::Duration,
    Scalar::Period,
];

const CHARS: [char; 8] = [
    '\0',
    'a',
    'z',
    '~',
    '\u{E9}',
    '\u{7FF}',
    '\u{4E2D}',
    '\u{1F600}',
];

fn ctors(fields: Vec<Vec<Type>>) -> Vec<Ctor> {
    fields
        .into_iter()
        .enumerate()
        .map(|(i, fields)| Ctor {
            id: CtorId(1024 + i as u32),
            fields,
        })
        .collect()
}

fn sum(fields: Vec<Vec<Type>>) -> Type {
    Type::Sum(ctors(fields))
}

fn nullary(n: usize) -> Type {
    sum(vec![Vec::new(); n])
}

fn option(t: Type) -> Type {
    sum(vec![Vec::new(), vec![t]])
}

fn s(x: Scalar) -> Type {
    Type::Scalar(x)
}

fn random_fields(rng: &mut Rng, depth: usize, lists: bool) -> Vec<Type> {
    (0..rng.below(4))
        .map(|_| random_type(rng, depth + 1, lists))
        .collect()
}

fn random_type(rng: &mut Rng, depth: usize, lists: bool) -> Type {
    if depth >= 3 || rng.chance(2) {
        return Type::Scalar(rng.pick(&SCALARS));
    }
    match rng.below(if lists { 8 } else { 7 }) {
        0 => Type::Record(random_fields(rng, depth, lists)),
        1 => {
            let mut fields = random_fields(rng, depth, lists);
            if fields.is_empty() {
                fields.push(random_type(rng, depth + 1, lists));
            }
            let mut cs = vec![Vec::new(), fields];
            if rng.chance(2) {
                cs.reverse();
            }
            sum(cs)
        }
        2 => nullary(1 + rng.below(5)),
        3 => sum(vec![random_fields(rng, depth, lists)]),
        4 if depth == 0 && rng.chance(4) => sum((0..UNION_LIMIT + 1 + rng.below(200))
            .map(|_| {
                if rng.chance(3) {
                    Vec::new()
                } else {
                    vec![Type::Scalar(rng.pick(&SCALARS))]
                }
            })
            .collect()),
        7 => Type::List(Box::new(random_type(rng, depth + 1, lists))),
        _ => sum((0..2 + rng.below(4))
            .map(|_| random_fields(rng, depth, lists))
            .collect()),
    }
}

fn random_text(rng: &mut Rng) -> String {
    (0..rng.below(6)).map(|_| rng.pick(&CHARS)).collect()
}

fn random_bytes(rng: &mut Rng) -> Vec<u8> {
    (0..rng.below(9)).map(|_| rng.next() as u8).collect()
}

fn random_scalar(rng: &mut Rng, x: Scalar) -> Value {
    let r = rng.next();
    match x {
        Scalar::I8 => Value::I8(r as i8),
        Scalar::I16 => Value::I16(r as i16),
        Scalar::I32 => Value::I32(r as i32),
        Scalar::I64 => Value::I64(r as i64),
        Scalar::F32 => Value::F32(F32::new(f32::from_bits(r as u32))),
        Scalar::F64 => Value::F64(F64::new(f64::from_bits(r))),
        Scalar::Decimal(_) => Value::Decimal(i128::from(r as i64) * 1_000_003),
        Scalar::Text => Value::Text(random_text(rng)),
        Scalar::Bytes => Value::Bytes(random_bytes(rng)),
        Scalar::SortKey => Value::SortKey(random_bytes(rng)),
        Scalar::Instant => Value::Instant(r as i64),
        Scalar::Date => Value::Date(r as i32),
        Scalar::Time => Value::Time((r % 86_400_000_000) as i64),
        Scalar::DateTime => Value::DateTime(r as i64),
        Scalar::ZonedDateTime => Value::ZonedDateTime {
            local: r as i64,
            zone: random_text(rng),
        },
        Scalar::Duration => Value::Duration(r as i64),
        Scalar::Period => Value::Period {
            months: r as i32,
            days: (r >> 32) as i32,
        },
    }
}

fn random_datum(rng: &mut Rng, t: &Type) -> Datum {
    match t {
        Type::Scalar(x) => Datum::Scalar(random_scalar(rng, *x)),
        Type::Record(ts) => Datum::Record(ts.iter().map(|t| random_datum(rng, t)).collect()),
        Type::Sum(cs) => {
            let ctor = rng.below(cs.len());
            Datum::Sum {
                ctor: ctor as u32,
                fields: cs[ctor]
                    .fields
                    .iter()
                    .map(|t| random_datum(rng, t))
                    .collect(),
            }
        }
        Type::List(e) => Datum::List((0..rng.below(4)).map(|_| random_datum(rng, e)).collect()),
    }
}

fn to_value(d: Datum) -> Value {
    match d {
        Datum::Scalar(v) => v,
        Datum::Record(fs) => Value::Record(fs.into_iter().map(to_value).collect()),
        Datum::Sum { ctor, fields } => Value::Sum {
            ctor,
            fields: fields.into_iter().map(to_value).collect(),
        },
        Datum::List(_) => panic!("a key value holds no list"),
    }
}

fn check_buffer(b: &Buffer) {
    let padded = b.padded();
    assert_eq!(b.as_slice(), &padded[..b.len()]);
    assert!(padded.len().is_multiple_of(ALIGN));
    assert!(padded[b.len()..].iter().all(|&x| x == 0), "padding is zero");
    if !padded.is_empty() {
        assert!(padded.as_ptr().addr().is_multiple_of(ALIGN), "aligned");
    }
}

fn i32s(b: &Buffer) -> Vec<i32> {
    b.as_slice()
        .chunks(4)
        .map(|c| i32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

fn check_offsets(offsets: &Buffer, len: usize, end: usize) {
    check_buffer(offsets);
    let o = i32s(offsets);
    assert_eq!(len + 1, o.len());
    assert_eq!(0, o[0]);
    assert!(o.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(end, o[len] as usize);
}

fn check_fields(types: &[Type], col: &Column) {
    let Column::Struct { len, children } = col else {
        panic!("{col:?} is not a struct");
    };
    assert_eq!(types.len(), children.len());
    for (t, c) in types.iter().zip(children) {
        assert_eq!(*len, c.len());
        check_column(t, c);
    }
}

fn check_scalar(x: Scalar, col: &Column) {
    match x {
        Scalar::Text | Scalar::Bytes | Scalar::SortKey => {
            let Column::Binary { len, offsets, data } = col else {
                panic!("{x:?} is not binary: {col:?}");
            };
            check_buffer(data);
            check_offsets(offsets, *len, data.len());
        }
        Scalar::ZonedDateTime => check_fields(&[s(Scalar::I64), s(Scalar::Text)], col),
        Scalar::Period => check_fields(&[s(Scalar::I32), s(Scalar::I32)], col),
        _ => {
            let Column::Fixed { len, width, data } = col else {
                panic!("{x:?} is not fixed: {col:?}");
            };
            let expected = match x {
                Scalar::I8 => 1,
                Scalar::I16 => 2,
                Scalar::I32 | Scalar::F32 | Scalar::Date => 4,
                Scalar::Decimal(_) => 16,
                _ => 8,
            };
            assert_eq!(expected, *width);
            assert_eq!(len * width, data.len());
            check_buffer(data);
        }
    }
}

fn check_union(cs: &[Ctor], col: &Column) {
    let Column::Union {
        len,
        type_ids,
        offsets,
        children,
    } = col
    else {
        panic!("a sum of {} is not a union: {col:?}", cs.len());
    };
    check_buffer(type_ids);
    check_buffer(offsets);
    assert_eq!(*len, type_ids.len());
    assert_eq!(*len, i32s(offsets).len());
    assert!(children.len() <= UNION_LIMIT);
    let mut next = vec![0; children.len()];
    for (&id, &at) in type_ids.as_slice().iter().zip(&i32s(offsets)) {
        let id = usize::from(id);
        assert!(id < children.len());
        assert_eq!(next[id], at as usize, "offsets count each child's rows");
        next[id] += 1;
    }
    for (n, c) in next.iter().zip(children) {
        assert_eq!(*n, c.len());
    }
    if cs.len() <= UNION_LIMIT {
        assert_eq!(cs.len(), children.len());
        for (c, child) in cs.iter().zip(children) {
            check_fields(&c.fields, child);
        }
    } else {
        let full = leaf_count(&children[0]);
        assert!(full >= UNION_LIMIT && is_power_of_limit(full));
        assert!(
            full * UNION_LIMIT >= cs.len(),
            "the union nests no deeper than needed"
        );
        let mut first = 0;
        for (i, child) in children.iter().enumerate() {
            let leaves = leaf_count(child);
            if i + 1 < children.len() {
                assert_eq!(full, leaves);
            }
            check_union(&cs[first..first + leaves], child);
            first += leaves;
        }
        assert_eq!(cs.len(), first);
    }
}

fn is_power_of_limit(mut n: usize) -> bool {
    while n.is_multiple_of(UNION_LIMIT) {
        n /= UNION_LIMIT;
    }
    n == 1
}

fn depth(col: &Column) -> usize {
    match col {
        Column::Union { children, .. } => 1 + children.first().map_or(0, depth),
        _ => 0,
    }
}

fn leaf_count(col: &Column) -> usize {
    match col {
        Column::Union { children, .. } => children.iter().map(leaf_count).sum(),
        _ => 1,
    }
}

fn check_column(t: &Type, col: &Column) {
    match t {
        Type::Scalar(x) => check_scalar(*x, col),
        Type::Record(ts) => check_fields(ts, col),
        Type::List(e) => {
            let Column::List {
                len,
                offsets,
                child,
            } = col
            else {
                panic!("a list is not a list column: {col:?}");
            };
            check_offsets(offsets, *len, child.len());
            check_column(e, child);
        }
        Type::Sum(cs) => match shape(cs) {
            Shape::Payload => check_fields(&cs[0].fields, col),
            Shape::Tag => {
                let Column::Fixed { len, width, data } = col else {
                    panic!("an enum is not a tag vector: {col:?}");
                };
                assert_eq!(tag_width(cs.len()), *width);
                assert_eq!(len * width, data.len());
                check_buffer(data);
            }
            Shape::Option { none } => {
                let Column::Nullable { validity, inner } = col else {
                    panic!("an option is not nullable: {col:?}");
                };
                check_buffer(validity);
                assert_eq!(inner.len().div_ceil(8), validity.len());
                match cs[1 - none].fields.as_slice() {
                    [t] => check_column(t, inner),
                    ts => check_fields(ts, inner),
                }
            }
            Shape::Union => check_union(cs, col),
        },
    }
}

fn check_batch(b: &Batch) {
    for (t, c) in b.schema().fields().iter().zip(b.columns()) {
        assert_eq!(b.len(), c.len());
        check_column(t, c);
    }
}

fn has_list(t: &Type) -> bool {
    match t {
        Type::Scalar(_) => false,
        Type::Record(ts) => ts.iter().any(has_list),
        Type::Sum(cs) => cs.iter().any(|c| c.fields.iter().any(has_list)),
        Type::List(_) => true,
    }
}

fn widest(t: &Type) -> usize {
    match t {
        Type::Scalar(_) => 0,
        Type::Record(ts) => ts.iter().map(widest).max().unwrap_or(0),
        Type::Sum(cs) => cs
            .iter()
            .flat_map(|c| c.fields.iter().map(widest))
            .fold(cs.len(), usize::max),
        Type::List(e) => widest(e),
    }
}

fn random_schema(rng: &mut Rng, lists: bool) -> Schema {
    Schema::new(
        (0..=rng.below(4))
            .map(|_| random_type(rng, 0, lists))
            .collect(),
    )
}

#[test]
fn random_value_rows_round_trip() {
    let mut rng = Rng(1);
    let mut wide = 0;
    for _ in 0..400 {
        let schema = random_schema(&mut rng, false);
        wide += usize::from(schema.fields().iter().any(|t| widest(t) > UNION_LIMIT));
        let rows: Vec<Vec<Value>> = (0..rng.below(40))
            .map(|_| {
                schema
                    .fields()
                    .iter()
                    .map(|t| to_value(random_datum(&mut rng, t)))
                    .collect()
            })
            .collect();
        let batch = Batch::from_rows(schema, rows.iter().map(Vec::as_slice));
        check_batch(&batch);
        assert_eq!(rows.len(), batch.len());
        for (i, r) in rows.iter().enumerate() {
            assert_eq!(*r, batch.row(i));
        }
    }
    assert!(
        wide > 10,
        "only {wide} schemas had a sum past the union limit"
    );
}

#[test]
fn random_nested_rows_round_trip() {
    let mut rng = Rng(2);
    let mut with_lists = 0;
    for _ in 0..400 {
        let schema = random_schema(&mut rng, true);
        with_lists += usize::from(schema.fields().iter().any(has_list));
        let rows: Vec<Vec<Datum>> = (0..rng.below(30))
            .map(|_| {
                schema
                    .fields()
                    .iter()
                    .map(|t| random_datum(&mut rng, t))
                    .collect()
            })
            .collect();
        let mut b = BatchBuilder::new(schema);
        for r in &rows {
            b.push_datums(r).expect("a test row fits a batch");
        }
        let batch = b.finish();
        check_batch(&batch);
        for (i, r) in rows.iter().enumerate() {
            assert_eq!(*r, batch.datums(i));
        }
    }
    assert!(with_lists > 50, "only {with_lists} schemas had lists");
}

#[test]
fn a_cloned_batch_keeps_its_alignment_and_rows() {
    let mut rng = Rng(3);
    let schema = Schema::new(vec![s(Scalar::Text), option(s(Scalar::I64))]);
    let rows: Vec<Vec<Value>> = (0..100)
        .map(|_| {
            schema
                .fields()
                .iter()
                .map(|t| to_value(random_datum(&mut rng, t)))
                .collect()
        })
        .collect();
    let batch = Batch::from_rows(schema, rows.iter().map(Vec::as_slice));
    let copy = batch.clone();
    check_batch(&copy);
    assert_eq!(batch, copy);
}

#[test]
fn a_buffer_stays_aligned_and_padded_as_it_grows() {
    let mut b = Buffer::new();
    assert!(b.padded().is_empty());
    for i in 0..1000u32 {
        b.extend(&i.to_le_bytes()[..=((i % 4) as usize)]);
        check_buffer(&b);
    }
    let len = b.len();
    let copy = b.as_slice().to_vec();
    let (bytes, range) = b.into_parts();
    assert_eq!(len, range.len());
    assert!(bytes[range.start..].as_ptr().addr().is_multiple_of(ALIGN));
    assert_eq!(copy, bytes[range]);
}

fn build(t: Type, values: &[Value]) -> Batch {
    let rows: Vec<Vec<Value>> = values.iter().map(|v| vec![v.clone()]).collect();
    let batch = Batch::from_rows(Schema::new(vec![t]), rows.iter().map(Vec::as_slice));
    check_batch(&batch);
    for (i, v) in values.iter().enumerate() {
        assert_eq!(*v, batch.value(0, i));
    }
    batch
}

fn sv(ctor: u32, fields: Vec<Value>) -> Value {
    Value::Sum { ctor, fields }
}

fn wide_sum(n: usize) -> Type {
    sum((0..n)
        .map(|i| {
            if i % 5 == 0 {
                Vec::new()
            } else {
                vec![s(Scalar::I32)]
            }
        })
        .collect())
}

fn wide_value(at: usize) -> Value {
    if at.is_multiple_of(5) {
        sv(at as u32, vec![])
    } else {
        sv(at as u32, vec![Value::I32(at as i32)])
    }
}

#[test]
fn a_sum_past_127_squared_constructors_nests_three_deep() {
    let n = UNION_LIMIT * UNION_LIMIT + 1;
    let batch = build(
        wide_sum(n),
        &[
            wide_value(n - 1),
            wide_value(0),
            wide_value(UNION_LIMIT * UNION_LIMIT - 1),
        ],
    );
    let col = &batch.columns()[0];
    assert_eq!(3, depth(col));
    let Column::Union {
        type_ids, children, ..
    } = col
    else {
        panic!("not a union");
    };
    assert_eq!(2, children.len());
    assert_eq!(&[1, 0, 0], type_ids.as_slice());
}

#[test]
fn an_offset_is_appended_up_to_i32_max_and_refused_past_it() {
    let mut offsets = Buffer::new();
    assert_eq!(Ok(()), column::push_offset(&mut offsets, i32::MAX as usize));
    assert_eq!(Err(TooLarge), column::push_offset(&mut offsets, 1 << 31));
    assert_eq!(i32::MAX.to_le_bytes().as_slice(), offsets.as_slice());
}

#[test]
fn a_batch_is_cut_before_the_row_that_would_pass_an_offset_and_not_at_it() {
    column::OFFSET_LIMIT.set(10);
    let schema = Schema::new(vec![s(Scalar::I64), s(Scalar::Text)]);
    let row = |i: i64, t: &str| vec![Value::I64(i), Value::Text(t.into())];
    let rows = [row(1, "abcd"), row(2, "efgh"), row(3, "ab"), row(4, "c")];
    let mut b = BatchBuilder::new(schema.clone());
    let mut cuts = Vec::new();
    for r in &rows {
        cuts.push(
            b.push_row_or_cut(r)
                .map(|c| c.map(|c| (0..c.len()).map(|i| c.row(i)).collect::<Vec<_>>())),
        );
    }
    assert_eq!(
        vec![Ok(None), Ok(None), Ok(None), Ok(Some(rows[..3].to_vec()))],
        cuts
    );
    let rest = b.finish();
    check_batch(&rest);
    assert_eq!(
        vec![rows[3].clone()],
        (0..rest.len()).map(|i| rest.row(i)).collect::<Vec<_>>()
    );
    let mut alone = BatchBuilder::new(schema);
    let empty = alone.bytes();
    assert_eq!(Err(TooLarge), alone.push_row_or_cut(&row(5, "abcdefghijk")));
    assert_eq!((0, empty), (alone.len(), alone.bytes()));
}

#[test]
fn a_cut_batch_and_the_rows_after_it_hold_every_row_of_every_column_shape_in_order() {
    let mut rng = Rng(3);
    let (mut cut, mut alone) = (0, 0);
    for _ in 0..400 {
        let limit = 8 + rng.below(56);
        column::OFFSET_LIMIT.set(limit);
        let schema = random_schema(&mut rng, true);
        let mut b = BatchBuilder::new(schema.clone());
        let (mut batches, mut kept) = (Vec::new(), Vec::new());
        for _ in 0..rng.below(30) {
            let r: Vec<Datum> = schema
                .fields()
                .iter()
                .map(|t| random_datum(&mut rng, t))
                .collect();
            let held = (b.len(), b.bytes());
            match b.push_datums_or_cut(&r) {
                Ok(full) => {
                    cut += usize::from(full.is_some());
                    batches.extend(full);
                    kept.push(r);
                }
                Err(TooLarge) => {
                    alone += 1;
                    assert_eq!(held, (b.len(), b.bytes()));
                }
            }
        }
        batches.push(b.finish());
        let mut got = Vec::new();
        for batch in &batches {
            check_batch(batch);
            got.extend((0..batch.len()).map(|i| batch.datums(i)));
        }
        assert_eq!(kept, got);
    }
    assert!(cut > 100, "only {cut} batches were cut");
    assert!(alone > 10, "only {alone} rows passed the limit alone");
}
