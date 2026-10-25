use crate::key::{CtorId, F32, F64, Scalar, Tid, Value};
use crate::storage::{self, PageKind};

use super::dir::{category, need_category};
use super::page::{self, Arrangement, Slot};
use super::row::{CtorType, FieldType, Layout, Pointer, Stored};
use super::version::{Flags, Header, tid_bits, tid_from_bits};
use super::{fixed, slotted};

const PAGE: usize = 4096;

fn scalar(s: Scalar) -> FieldType {
    FieldType::Scalar(s)
}

fn heap_page(arrangement: Arrangement, position: u32) -> Vec<u8> {
    let mut p = vec![0; PAGE];
    p[storage::FIRST_EDITABLE..storage::HEADER_LEN].copy_from_slice(&storage::header_edit(
        1,
        arrangement.kind(),
        7,
        0,
    ));
    arrangement.init(&mut p, position);
    p
}

fn version(xmin: u64, body: &[u8]) -> Vec<u8> {
    let mut v = vec![0; 24];
    Header::new(xmin).write(&mut v);
    v.extend_from_slice(body);
    v
}

#[test]
fn header_round_trips_and_zero_next_is_none() {
    let h = Header {
        xmin: 5,
        xmax: u64::MAX,
        next: Tid::new((1 << 48) - 1, 65535),
    };
    let mut b = [0; 24];
    h.write(&mut b);
    assert_eq!(h, Header::read(&b));
    assert_eq!(None, Header::read(&[0; 24]).next);
    let t = Tid::new(0x1234_5678_9abc, 7).unwrap();
    assert_eq!(0x1234_5678_9abc_0007, tid_bits(t));
    assert_eq!(Some(t), tid_from_bits(tid_bits(t)));
    assert_eq!(None, tid_from_bits(0));
}

#[test]
fn flags_combine() {
    let f = Flags::MOVED.with(Flags::HEAP_ONLY);
    assert!(f.contains(Flags::MOVED));
    assert!(!f.contains(Flags::HOT_UPDATED));
    assert_eq!(Flags::HEAP_ONLY, f.without(Flags::MOVED));
}

#[test]
fn fixed_page_fills_every_slot_then_refuses() {
    let width = 40;
    let mut p = heap_page(Arrangement::Fixed { width }, 3);
    let n = fixed::capacity(PAGE, width);
    assert_eq!(n, page::slot_count(&p));
    assert_eq!(3, page::position(&p));
    assert_eq!(Arrangement::Fixed { width }, page::arrangement(&p));
    for i in 0..n {
        assert_eq!(n * width - i * width, page::free(&p));
        let v = version(i as u64 + 1, &[i as u8; 8]);
        assert_eq!(Some(i), page::insert(&mut p, &v, Flags(i as u16)));
    }
    assert_eq!(0, page::free(&p));
    assert_eq!(None, page::insert(&mut p, &version(9, &[]), Flags::NONE));
    let Slot::Normal { flags, at } = page::get(&p, 17) else {
        panic!("slot 17 is empty");
    };
    assert_eq!(Flags(17), flags);
    assert_eq!(width, at.len());
    assert_eq!(18, Header::read(&p[at.clone()]).xmin);
    assert_eq!([17; 8], p[at.start + 24..at.start + 32]);
    assert_eq!(Slot::Unused, page::get(&p, n));
}

#[test]
fn fixed_capacity_leaves_room_for_the_bitmap_and_flags() {
    let end =
        |n: usize, width: usize| (32 + 8 * n.div_ceil(64) + 2 * n).div_ceil(8) * 8 + n * width;
    for width in [24, 25, 32, 100, 1000, 4000, 4048] {
        let n = fixed::capacity(PAGE, width);
        assert!(n > 0, "{width}");
        assert!(end(n, width) <= PAGE, "{width}");
        assert!(end(n + 1, width) > PAGE, "{width}");
    }
    assert_eq!(0, fixed::capacity(PAGE, PAGE));
}

#[test]
fn slotted_page_takes_versions_until_full() {
    let mut p = heap_page(Arrangement::Slotted, 0);
    let mut inserted = Vec::new();
    let mut i = 0u8;
    loop {
        let v = version(u64::from(i), &vec![i; 1 + usize::from(i) * 7 % 90]);
        let before = page::free(&p);
        match page::insert(&mut p, &v, Flags::HEAP_ONLY) {
            Some(slot) => {
                assert_eq!(inserted.len(), slot);
                inserted.push(v);
            }
            None => {
                assert!(before < Arrangement::Slotted.need(v.len()));
                break;
            }
        }
        i += 1;
    }
    for (slot, v) in inserted.iter().enumerate() {
        let Slot::Normal { flags, at } = page::get(&p, slot) else {
            panic!("slot {slot} is empty");
        };
        assert_eq!(Flags::HEAP_ONLY, flags);
        assert_eq!(&v[..], &p[at]);
    }
}

#[test]
fn compaction_keeps_slots_and_bytes() {
    let mut p = heap_page(Arrangement::Slotted, 0);
    let versions: Vec<Vec<u8>> = (0..10).map(|i| version(i, &[i as u8; 50])).collect();
    for v in &versions {
        page::insert(&mut p, v, Flags::NONE).unwrap();
    }
    slotted::compact(&mut p);
    for (s, v) in versions.iter().enumerate() {
        let Slot::Normal { at, .. } = page::get(&p, s) else {
            panic!("slot {s} is empty");
        };
        assert_eq!(&v[..], &p[at]);
    }
}

#[test]
fn slotted_put_fills_gaps_and_keeps_dead_and_redirect_slots() {
    let mut p = heap_page(Arrangement::Slotted, 0);
    let v = version(1, b"abc");
    let normal = Slot::Normal {
        flags: Flags::MOVED,
        at: 0..0,
    };
    assert!(page::put(&mut p, 4, &Slot::Dead, &[]));
    assert!(page::put(&mut p, 2, &normal, &v));
    assert!(page::put(&mut p, 0, &Slot::Redirect(2), &[]));
    assert_eq!(5, page::slot_count(&p));
    assert_eq!(Slot::Redirect(2), page::get(&p, 0));
    assert_eq!(Slot::Unused, page::get(&p, 1));
    assert_eq!(Slot::Dead, page::get(&p, 4));
    assert!(!page::put(&mut p, 4, &Slot::Dead, &[]));
    assert_eq!(Some(1), page::insert(&mut p, &v, Flags::NONE));
    assert_eq!(Some(3), page::insert(&mut p, &v, Flags::NONE));
    assert_eq!(Some(5), page::insert(&mut p, &v, Flags::NONE));
}

#[test]
fn a_fixed_page_holds_no_dead_or_redirect_slot() {
    let mut p = heap_page(Arrangement::Fixed { width: 32 }, 0);
    assert!(!page::put(&mut p, 0, &Slot::Dead, &[]));
    assert!(!page::put(&mut p, 0, &Slot::Redirect(1), &[]));
}

#[test]
fn a_dead_slot_frees_its_tuple_space() {
    let mut p = heap_page(Arrangement::Slotted, 0);
    let big = version(1, &[7; 900]);
    let mut slots = Vec::new();
    while let Some(s) = page::insert(&mut p, &big, Flags::NONE) {
        slots.push(s);
    }
    assert_eq!(4, slots.len());
    let mut rebuilt = heap_page(Arrangement::Slotted, 0);
    for &s in &slots {
        let Slot::Normal { at, .. } = page::get(&p, s) else {
            panic!("slot {s} is empty");
        };
        let content = if s == 1 { Slot::Dead } else { page::get(&p, s) };
        assert!(page::put(&mut rebuilt, s, &content, &p[at]));
    }
    assert_eq!(Some(4), page::insert(&mut rebuilt, &big, Flags::NONE));
    assert_eq!(Slot::Dead, page::get(&rebuilt, 1));
}

#[test]
fn categories_never_promise_more_than_the_page_has() {
    for page_size in [4096, 8192, 65536] {
        for free in (0..page_size).step_by(7) {
            let c = category(free, page_size);
            for need in (1..page_size).step_by(13) {
                if u16::from(c) >= need_category(need, page_size) {
                    assert!(free >= need, "{page_size} {free} {need}");
                }
            }
        }
    }
}

fn bool_type() -> FieldType {
    FieldType::Sum(vec![
        CtorType {
            id: CtorId(1),
            fields: vec![],
        },
        CtorType {
            id: CtorId(2),
            fields: vec![],
        },
    ])
}

#[test]
fn fixed_fields_are_placed_by_decreasing_alignment() {
    let layout = Layout::new(vec![
        bool_type(),
        scalar(Scalar::I64),
        scalar(Scalar::I16),
        scalar(Scalar::Decimal(2)),
    ]);
    assert!(layout.is_fixed());
    assert_eq!(32, layout.fixed_width());
    let row = vec![
        Value::Sum {
            ctor: 1,
            fields: vec![],
        },
        Value::I64(-2),
        Value::I16(0x0102),
        Value::Decimal(12345),
    ];
    let (fixed, vars) = layout.encode(&row);
    assert!(vars.is_empty());
    assert_eq!(27, fixed.len());
    assert_eq!(12345i128.to_le_bytes(), fixed[0..16]);
    assert_eq!((-2i64).to_le_bytes(), fixed[16..24]);
    assert_eq!([2, 1], fixed[24..26]);
    assert_eq!(2, fixed[26]);
    let body = layout.assemble(&fixed, &[]);
    let (f, v) = layout.split(&body);
    assert!(v.is_empty());
    assert_eq!(row, layout.decode(f, &[]));
}

fn status_type() -> FieldType {
    FieldType::Sum(vec![
        CtorType {
            id: CtorId(1100),
            fields: vec![],
        },
        CtorType {
            id: CtorId(1101),
            fields: vec![scalar(Scalar::Instant)],
        },
        CtorType {
            id: CtorId(1102),
            fields: vec![scalar(Scalar::Instant)],
        },
    ])
}

#[test]
fn a_small_fixed_sum_stores_its_tag_then_its_largest_payload() {
    let layout = Layout::new(vec![status_type()]);
    assert!(layout.is_fixed());
    assert_eq!(16, layout.fixed_width());
    let (fixed, _) = layout.encode(&[Value::Sum {
        ctor: 2,
        fields: vec![Value::Instant(99)],
    }]);
    assert_eq!(1102u16.to_le_bytes(), fixed[0..2]);
    assert_eq!([0; 6], fixed[2..8]);
    assert_eq!(99i64.to_le_bytes(), fixed[8..16]);
    let (fixed, _) = layout.encode(&[Value::Sum {
        ctor: 0,
        fields: vec![],
    }]);
    assert_eq!(1100u16.to_le_bytes(), fixed[0..2]);
}

#[test]
fn tag_width_follows_the_largest_constructor_id() {
    let sum = |ids: &[u32]| {
        FieldType::Sum(
            ids.iter()
                .map(|&i| CtorType {
                    id: CtorId(i),
                    fields: vec![],
                })
                .collect(),
        )
    };
    let width = |t| Layout::new(vec![t]).fixed_width();
    assert_eq!(1, width(sum(&[3, 255])));
    assert_eq!(2, width(sum(&[3, 256])));
    assert_eq!(4, width(sum(&[3, 65536])));
    assert_eq!(0, width(sum(&[70000])));
}

#[test]
fn a_sum_too_large_for_the_fixed_part_keeps_only_its_tag_there() {
    let big = FieldType::Sum(vec![
        CtorType {
            id: CtorId(1024),
            fields: vec![scalar(Scalar::Decimal(0)); 5],
        },
        CtorType {
            id: CtorId(1025),
            fields: vec![],
        },
    ]);
    let layout = Layout::new(vec![big]);
    assert!(!layout.is_fixed());
    let (fixed, vars) = layout.encode(&[Value::Sum {
        ctor: 1,
        fields: vec![],
    }]);
    assert_eq!(1025u16.to_le_bytes(), fixed[..]);
    assert_eq!(vec![Vec::<u8>::new()], vars);
}

#[test]
fn a_sum_with_a_variable_payload_keeps_its_tag_fixed() {
    let shape = FieldType::Sum(vec![
        CtorType {
            id: CtorId(1024),
            fields: vec![scalar(Scalar::Text), scalar(Scalar::I32)],
        },
        CtorType {
            id: CtorId(1025),
            fields: vec![],
        },
    ]);
    let layout = Layout::new(vec![scalar(Scalar::I8), shape]);
    assert!(!layout.is_fixed());
    let row = vec![
        Value::I8(-1),
        Value::Sum {
            ctor: 0,
            fields: vec![Value::Text("héllo".into()), Value::I32(7)],
        },
    ];
    let (fixed, vars) = layout.encode(&row);
    assert_eq!(1024u16.to_le_bytes(), fixed[0..2]);
    assert_eq!(0xff, fixed[2]);
    let mut payload = vec![6];
    payload.extend_from_slice("héllo".as_bytes());
    payload.extend_from_slice(&7i32.to_le_bytes());
    assert_eq!(vec![payload], vars);
    let body = layout.assemble(&fixed, &inline(vars.clone()));
    let (f, stored) = layout.split(&body);
    assert_eq!(inline(vars.clone()), stored);
    assert_eq!(row, layout.decode(f, &vars));
}

fn every_type() -> Vec<FieldType> {
    use Scalar::*;
    let tree = FieldType::Sum(vec![
        CtorType {
            id: CtorId(5000),
            fields: vec![scalar(Text)],
        },
        CtorType {
            id: CtorId(300),
            fields: vec![
                FieldType::Record(vec![scalar(Bytes), scalar(I16)]),
                status_type(),
            ],
        },
    ]);
    vec![
        scalar(I8),
        scalar(I16),
        scalar(I32),
        scalar(I64),
        scalar(F32),
        scalar(F64),
        scalar(Decimal(4)),
        scalar(Text),
        scalar(Bytes),
        scalar(SortKey),
        scalar(Instant),
        scalar(Date),
        scalar(Time),
        scalar(DateTime),
        scalar(ZonedDateTime),
        scalar(Duration),
        scalar(Period),
        FieldType::Record(vec![scalar(I8), scalar(Decimal(0)), bool_type()]),
        FieldType::Record(vec![scalar(Text), scalar(Date)]),
        tree,
        status_type(),
    ]
}

fn every_value(n: i64) -> Vec<Value> {
    let tree = if n % 2 == 0 {
        Value::Sum {
            ctor: 0,
            fields: vec![Value::Text(format!("leaf {n}"))],
        }
    } else {
        Value::Sum {
            ctor: 1,
            fields: vec![
                Value::Record(vec![
                    Value::Bytes(vec![n as u8; n as usize]),
                    Value::I16(-3),
                ]),
                Value::Sum {
                    ctor: 1,
                    fields: vec![Value::Instant(n)],
                },
            ],
        }
    };
    vec![
        Value::I8(n as i8),
        Value::I16(-(n as i16)),
        Value::I32(n as i32 * 1000),
        Value::I64(i64::MIN + n),
        Value::F32(F32::new(-0.0)),
        Value::F64(F64::new(f64::NAN)),
        Value::Decimal(-(n as i128) * 10_000_000_000_000_000),
        Value::Text("x".repeat(n as usize)),
        Value::Bytes(vec![0xab; 2 * n as usize]),
        Value::SortKey(vec![1, 2, 3]),
        Value::Instant(n * 1_000_000),
        Value::Date(-(n as i32)),
        Value::Time(86_399_999_999),
        Value::DateTime(n),
        Value::ZonedDateTime {
            local: n,
            zone: "Europe/Lisbon".into(),
        },
        Value::Duration(-n),
        Value::Period {
            months: 14,
            days: -(n as i32),
        },
        Value::Record(vec![
            Value::I8(1),
            Value::Decimal(n as i128),
            Value::Sum {
                ctor: (n % 2) as u32,
                fields: vec![],
            },
        ]),
        Value::Record(vec![Value::Text(String::new()), Value::Date(3)]),
        tree,
        Value::Sum {
            ctor: 0,
            fields: vec![],
        },
    ]
}

fn inline(vars: Vec<Vec<u8>>) -> Vec<Stored> {
    vars.into_iter().map(Stored::Inline).collect()
}

#[test]
fn every_field_type_round_trips() {
    let layout = Layout::new(every_type());
    for n in 0..20 {
        let row = every_value(n);
        let (fixed, vars) = layout.encode(&row);
        let body = layout.assemble(&fixed, &inline(vars.clone()));
        assert_eq!(layout.body_len(&inline(vars.clone())), body.len());
        let (f, stored) = layout.split(&body);
        assert_eq!(inline(vars.clone()), stored);
        assert_eq!(row, layout.decode(f, &vars), "row {n}");
    }
}

#[test]
fn out_of_line_fields_are_marked_and_keep_their_neighbours() {
    let layout = Layout::new(vec![
        scalar(Scalar::Text),
        scalar(Scalar::I64),
        scalar(Scalar::Bytes),
        scalar(Scalar::Text),
    ]);
    let row = vec![
        Value::Text("a".into()),
        Value::I64(4),
        Value::Bytes(vec![9; 3000]),
        Value::Text("z".into()),
    ];
    let (fixed, vars) = layout.encode(&row);
    let p = Pointer {
        len: 3000,
        first: Tid::new(77, 3).unwrap(),
    };
    let stored = vec![
        Stored::Inline(vars[0].clone()),
        Stored::Out(p),
        Stored::Inline(vars[2].clone()),
    ];
    let body = layout.assemble(&fixed, &stored);
    assert_eq!(8 + 6 + 1 + 1 + 16 + 1, body.len());
    assert_eq!(stored, layout.split(&body).1);
    assert_eq!(row, layout.decode(&fixed, &vars));
}

#[test]
#[should_panic(expected = "is not of field type")]
fn a_row_of_the_wrong_type_is_refused() {
    Layout::new(vec![scalar(Scalar::I64)]).encode(&[Value::I32(1)]);
}

#[test]
fn heap_pages_name_their_kind() {
    let p = heap_page(Arrangement::Slotted, 0);
    assert_eq!(PageKind::HeapSlotted as u16, storage::kind(&p));
    let p = heap_page(Arrangement::Fixed { width: 48 }, 0);
    assert_eq!(PageKind::HeapFixed as u16, storage::kind(&p));
}

#[test]
fn a_64_kib_page_holds_its_largest_version() {
    let size = 65536;
    let mut p = vec![0; size];
    p[storage::FIRST_EDITABLE..storage::HEADER_LEN].copy_from_slice(&storage::header_edit(
        1,
        PageKind::HeapSlotted,
        7,
        0,
    ));
    Arrangement::Slotted.init(&mut p, 0);
    let v = vec![5; slotted::max_version(size)];
    assert_eq!(v.len(), page::free(&p));
    assert_eq!(Some(0), page::insert(&mut p, &v, Flags::NONE));
    let Slot::Normal { at, .. } = page::get(&p, 0) else {
        panic!("slot 0 is empty");
    };
    assert_eq!(&v[..], &p[at]);
    assert_eq!(0, page::free(&p));
    assert_eq!(None, page::insert(&mut p, &[1; 8], Flags::NONE));
}
