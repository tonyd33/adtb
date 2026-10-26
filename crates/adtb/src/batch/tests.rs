use crate::heap::{self, FieldType};
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
        rows.iter().for_each(|r| b.push_datums(r));
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

fn one(t: Type) -> Column {
    BatchBuilder::new(Schema::new(vec![t]))
        .finish()
        .into_columns()
        .remove(0)
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

#[test]
fn an_option_of_a_scalar_is_the_scalar_with_a_validity_bitmap() {
    let t = option(s(Scalar::I64));
    assert_eq!(Shape::Option { none: 0 }, shape(ctors_of(&t)));
    let values: Vec<Value> = (0..10)
        .map(|i| {
            if i % 3 == 0 {
                sv(0, vec![])
            } else {
                sv(1, vec![Value::I64(i)])
            }
        })
        .collect();
    let batch = build(t, &values);
    let Column::Nullable { validity, inner } = &batch.columns()[0] else {
        panic!("not nullable");
    };
    assert_eq!(&[0b1011_0110, 0b01], validity.as_slice());
    assert!(matches!(
        **inner,
        Column::Fixed {
            width: 8,
            len: 10,
            ..
        }
    ));
}

fn ctors_of(t: &Type) -> &[Ctor] {
    let Type::Sum(cs) = t else {
        panic!("{t:?} is not a sum");
    };
    cs
}

#[test]
fn the_nullary_constructor_of_an_option_may_come_second() {
    let t = sum(vec![vec![s(Scalar::Text)], Vec::new()]);
    assert_eq!(Shape::Option { none: 1 }, shape(ctors_of(&t)));
    let batch = build(
        t,
        &[
            sv(1, vec![]),
            sv(0, vec![Value::Text("x".into())]),
            sv(1, vec![]),
        ],
    );
    let Column::Nullable { validity, .. } = &batch.columns()[0] else {
        panic!("not nullable");
    };
    assert_eq!(&[0b010], validity.as_slice());
}

#[test]
fn an_option_of_several_fields_is_a_struct_with_a_validity_bitmap() {
    let t = sum(vec![Vec::new(), vec![s(Scalar::I32), s(Scalar::Text)]]);
    assert_eq!(Shape::Option { none: 0 }, shape(ctors_of(&t)));
    let col = one(t);
    let Column::Nullable { inner, .. } = col else {
        panic!("not nullable");
    };
    assert!(matches!(*inner, Column::Struct { ref children, .. } if children.len() == 2));
}

#[test]
fn an_option_takes_a_validity_bitmap_only_where_its_payload_has_none() {
    let record = sum(vec![vec![s(Scalar::I8), s(Scalar::I8)]]);
    let enumeration = nullary(3);
    let union = sum(vec![vec![s(Scalar::I8)], vec![s(Scalar::Text)]]);
    for (inner, shaped) in [
        (s(Scalar::I8), true),
        (Type::Record(vec![s(Scalar::I8)]), true),
        (Type::List(Box::new(s(Scalar::I8))), true),
        (record, true),
        (enumeration, true),
        (union, false),
        (option(s(Scalar::I8)), false),
    ] {
        let t = option(inner.clone());
        let expected = if shaped {
            Shape::Option { none: 0 }
        } else {
            Shape::Union
        };
        assert_eq!(expected, shape(ctors_of(&t)), "Option of {inner:?}");
    }
    let nested = option(option(s(Scalar::I8)));
    let batch = build(
        nested,
        &[
            sv(0, vec![]),
            sv(1, vec![sv(0, vec![])]),
            sv(1, vec![sv(1, vec![Value::I8(4)])]),
        ],
    );
    let Column::Union { children, .. } = &batch.columns()[0] else {
        panic!("Option (Option I8) is not a union");
    };
    assert!(
        matches!(children[1], Column::Struct { ref children, .. } if matches!(children[0], Column::Nullable { .. }))
    );
}

#[test]
fn a_sum_of_nullary_constructors_is_its_tag_vector() {
    let bool_type = nullary(2);
    assert_eq!(Shape::Tag, shape(ctors_of(&bool_type)));
    let batch = build(bool_type, &[sv(1, vec![]), sv(0, vec![]), sv(1, vec![])]);
    let Column::Fixed { width, data, .. } = &batch.columns()[0] else {
        panic!("not a tag vector");
    };
    assert_eq!(1, *width);
    assert_eq!(&[1, 0, 1], data.as_slice());

    assert_eq!(1, tag_width(256));
    assert_eq!(2, tag_width(257));
    assert_eq!(2, tag_width(65536));
    assert_eq!(4, tag_width(65537));
    let wide = nullary(300);
    let batch = build(wide, &[sv(299, vec![]), sv(0, vec![]), sv(256, vec![])]);
    let Column::Fixed { width, data, .. } = &batch.columns()[0] else {
        panic!("not a tag vector");
    };
    assert_eq!(2, *width);
    assert_eq!(&[0x2b, 1, 0, 0, 0, 1], data.as_slice());
    assert_eq!(Shape::Tag, shape(&[]));
}

#[test]
fn a_one_constructor_sum_is_its_payload_with_no_tag() {
    let t = sum(vec![vec![s(Scalar::I16), s(Scalar::Text)]]);
    assert_eq!(Shape::Payload, shape(ctors_of(&t)));
    let batch = build(t, &[sv(0, vec![Value::I16(3), Value::Text("a".into())])]);
    assert!(
        matches!(&batch.columns()[0], Column::Struct { len: 1, children } if children.len() == 2)
    );

    let unit = nullary(1);
    assert_eq!(Shape::Payload, shape(ctors_of(&unit)));
    let batch = build(unit, &[sv(0, vec![]), sv(0, vec![])]);
    assert!(
        matches!(&batch.columns()[0], Column::Struct { len: 2, children } if children.is_empty())
    );
}

#[test]
fn a_general_sum_is_a_dense_union_of_compact_children() {
    let t = sum(vec![
        vec![s(Scalar::Text)],
        vec![s(Scalar::Text), s(Scalar::I32)],
        Vec::new(),
    ]);
    assert_eq!(Shape::Union, shape(ctors_of(&t)));
    let text = |x: &str| Value::Text(x.into());
    let batch = build(
        t,
        &[
            sv(0, vec![text("a")]),
            sv(1, vec![text("T1"), Value::I32(3)]),
            sv(1, vec![text("T2"), Value::I32(4)]),
            sv(2, vec![]),
            sv(0, vec![text("b")]),
        ],
    );
    let Column::Union {
        type_ids,
        offsets,
        children,
        ..
    } = &batch.columns()[0]
    else {
        panic!("not a union");
    };
    assert_eq!(&[0, 1, 1, 2, 0], type_ids.as_slice());
    assert_eq!(vec![0, 0, 1, 0, 1], i32s(offsets));
    assert_eq!(
        vec![2, 2, 1],
        children.iter().map(Column::len).collect::<Vec<_>>()
    );
}

#[test]
fn a_type_id_names_the_constructor_at_its_schema_position() {
    let cs = vec![
        Ctor {
            id: CtorId(1042),
            fields: vec![s(Scalar::I8)],
        },
        Ctor {
            id: CtorId(1040),
            fields: vec![s(Scalar::Text)],
        },
        Ctor {
            id: CtorId(1041),
            fields: Vec::new(),
        },
    ];
    let t = Type::Sum(cs);
    let batch = build(t, &[sv(1, vec![Value::Text("g".into())])]);
    let Column::Union { type_ids, .. } = &batch.columns()[0] else {
        panic!("not a union");
    };
    let Type::Sum(schema_ctors) = &batch.schema().fields()[0] else {
        panic!("not a sum");
    };
    assert_eq!(
        CtorId(1040),
        schema_ctors[usize::from(type_ids.as_slice()[0])].id
    );
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
fn a_sum_of_127_constructors_is_one_union() {
    let batch = build(wide_sum(127), &[wide_value(0), wide_value(126)]);
    let Column::Union {
        type_ids, children, ..
    } = &batch.columns()[0]
    else {
        panic!("not a union");
    };
    assert_eq!(127, children.len());
    assert_eq!(&[0, 126], type_ids.as_slice());
    assert!(children.iter().all(|c| matches!(c, Column::Struct { .. })));
}

#[test]
fn a_sum_past_127_constructors_is_a_union_of_unions() {
    let values: Vec<Value> = [0, 126, 127, 128, 253, 254, 299, 7]
        .into_iter()
        .map(wide_value)
        .collect();
    let batch = build(wide_sum(300), &values);
    let Column::Union {
        type_ids, children, ..
    } = &batch.columns()[0]
    else {
        panic!("not a union");
    };
    assert_eq!(3, children.len());
    assert_eq!(&[0, 0, 1, 1, 1, 2, 2, 0], type_ids.as_slice());
    assert_eq!(
        vec![127, 127, 46],
        children.iter().map(leaf_count).collect::<Vec<_>>()
    );
    let Column::Union {
        type_ids: inner, ..
    } = &children[1]
    else {
        panic!("not a union");
    };
    assert_eq!(&[0, 1, 126], inner.as_slice());

    let batch = build(wide_sum(128), &[wide_value(127), wide_value(0)]);
    let Column::Union { children, .. } = &batch.columns()[0] else {
        panic!("not a union");
    };
    assert_eq!(
        vec![127, 1],
        children.iter().map(leaf_count).collect::<Vec<_>>()
    );
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
fn heap_field_types_become_batch_types_with_their_constructor_ids() {
    let some = heap::CtorType {
        id: CtorId(7),
        fields: vec![FieldType::Scalar(Scalar::Text)],
    };
    let none = heap::CtorType {
        id: CtorId(6),
        fields: Vec::new(),
    };
    let row = vec![
        FieldType::Scalar(Scalar::I64),
        FieldType::Record(vec![FieldType::Scalar(Scalar::Date)]),
        FieldType::Sum(vec![none, some]),
    ];
    let schema = Schema::of_heap(&row);
    assert_eq!(
        &[
            s(Scalar::I64),
            Type::Record(vec![s(Scalar::Date)]),
            Type::Sum(vec![
                Ctor {
                    id: CtorId(6),
                    fields: Vec::new()
                },
                Ctor {
                    id: CtorId(7),
                    fields: vec![s(Scalar::Text)]
                },
            ]),
        ],
        schema.fields()
    );
    let rows = [
        vec![
            Value::I64(1),
            Value::Record(vec![Value::Date(3)]),
            sv(1, vec![Value::Text("a".into())]),
        ],
        vec![
            Value::I64(2),
            Value::Record(vec![Value::Date(-3)]),
            sv(0, vec![]),
        ],
    ];
    let batch = Batch::from_rows(schema, rows.iter().map(Vec::as_slice));
    check_batch(&batch);
    for (i, r) in rows.iter().enumerate() {
        assert_eq!(*r, batch.row(i));
    }
    assert!(matches!(batch.columns()[0], Column::Fixed { .. }));
}

#[test]
fn a_nested_result_is_a_list_of_structs() {
    let t = Type::List(Box::new(Type::Record(vec![
        s(Scalar::Text),
        Type::List(Box::new(s(Scalar::Text))),
    ])));
    let text = |x: &str| Datum::Scalar(Value::Text(x.into()));
    let row = vec![Datum::List(vec![
        Datum::Record(vec![
            text("ann"),
            Datum::List(vec![text("a@example.com"), text("g-123")]),
        ]),
        Datum::Record(vec![text("bob"), Datum::List(Vec::new())]),
    ])];
    let mut b = BatchBuilder::new(Schema::new(vec![t]));
    b.push_datums(&row);
    b.push_datums(&[Datum::List(Vec::new())]);
    let batch = b.finish();
    check_batch(&batch);
    assert_eq!(row, batch.datums(0));
    let Column::List { offsets, child, .. } = &batch.columns()[0] else {
        panic!("not a list");
    };
    assert_eq!(vec![0, 2, 2], i32s(offsets));
    assert!(matches!(**child, Column::Struct { len: 2, .. }));
}

#[test]
#[should_panic(expected = "holds no list")]
fn a_value_row_cannot_read_a_list() {
    let mut b = BatchBuilder::new(Schema::new(vec![Type::List(Box::new(s(Scalar::I8)))]));
    b.push_datums(&[Datum::List(Vec::new())]);
    b.finish().value(0, 0);
}

#[test]
#[should_panic(expected = "is not of type")]
fn a_row_of_the_wrong_type_is_refused() {
    let mut b = BatchBuilder::new(Schema::new(vec![s(Scalar::I64)]));
    b.push_row(&[Value::I32(1)]);
}
