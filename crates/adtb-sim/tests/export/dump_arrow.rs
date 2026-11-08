//! The data entries of a dump as Arrow IPC streams: what arrow-rs reads from them, and a restore
//! reading them and not the text form.

use std::collections::BTreeMap;
use std::io::Cursor;

use ::arrow_ipc::reader::StreamReader;
use adtb::batch::Datum;
use adtb::batch::ipc::{FileField, FileType};
use adtb::catalog::arrows::{self, file_schema};
use adtb::key::{F32, F64, Scalar, Value as Key};
use adtb::storage;
use adtb::{
    ArchiveTarget, CallOptions, Database, Error, Log, OpenOptions, RestorePoint, Value, restore_in,
};
use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::textual::read_document;
use adtb_lang::types::{RelId, Ty};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_simkit::api;
use adtb_simkit::open;
use adtb_simkit::search::Files;
use arrow_schema::{DataType, UnionMode};

use crate::arrow_ipc;

const FILE: &str = "app.adt";
const DUMP: &str = "d";
const EDITED: &str = "e";
const COPY: &str = "copy.adt";
const APP: &str = "module app
add module app
add type Color = Red | Green | Blue
add type Via = Email { addr: Text } | Phone { num: Text }
add type Wrap = Wrap { n: I32 }
add type Name = new Text
add type Point = { x: I16, y: I8 }
add type Item { id: I64 } =
    Thing { a: I8, b: I16, c: I32, d: I64, e: F32, f: F64, g: Decimal 4, h: Text, i: Bytes,
      j: SortKey, k: Instant, l: Date, m: Time, n: DateTime, o: ZonedDateTime, p: Duration,
      q: Period }
  | Rich { flag: Bool, opt: Option Text, opts: Option (Option I64), optv: Option Via,
      color: Color, via: Via, wrap: Wrap, name: Name, at: Point }
  | Empty
add relation items : Item key id
add type Ref = { id: Key refs, to: Key items }
add relation refs : Ref key id generate sequence
add type Flag = False | True
add type Maybe = None | Some { value: I64 }
add type Odd = { id: I64, b: Flag, o: Maybe }
add relation odds : Odd key id
";
/// The rows of each relation of `APP`, as documents.
const ROWS: [&str; 3] = [
    r#"data app::items
Thing { id: 1, a: -128, b: 300, c: -70000, d: 9000000000, e: 1.5, f: -0.25, g: 12.3456, h: "ä\n", i: 0x00ff, j: 0x2b4f01, k: 2026-10-25T09:30:00.000001Z, l: 2026-10-25, m: 09:30:00.000000, n: 2026-10-25T09:30:00.000000, o: 1970-01-01T00:00:00.000000[Asia/Tokyo], p: 90s, q: P1M-15D }
Thing { id: 2, a: 0, b: 0, c: 0, d: 0, e: 0.0, f: 0.0, g: 0.0000, h: "", i: 0x, j: 0x, k: 1970-01-01T00:00:00.000000Z, l: 1970-01-01, m: 00:00:00.000000, n: 1970-01-01T00:00:00.000000, o: 1970-01-02T00:00:00.000001[Europe/Paris], p: 0s, q: P0M0D }
Rich { id: 3, flag: True, opt: Some { value: "x" }, opts: Some { value: None }, optv: Some { value: Phone { num: "1" } }, color: Blue, via: Email { addr: "a@b" }, wrap: Wrap { n: 7 }, name: Name("n"), at: Point { x: -1, y: 2 } }
Rich { id: 4, flag: False, opt: None, opts: Some { value: Some { value: 5 } }, optv: None, color: Red, via: Phone { num: "2" }, wrap: Wrap { n: 0 }, name: Name(""), at: Point { x: 0, y: 0 } }
Empty { id: 5 }
"#,
    "data app::refs\nRef { id: refs.key(1), to: items.key(3) }\nRef { id: refs.key(2), to: items.key(5) }\n",
    "data app::odds\nOdd { id: 1, b: True, o: Some { value: 4 } }\nOdd { id: 2, b: False, o: None }\n",
];
const THING: &str = "data/app::items.Thing.arrows";

fn options() -> OpenOptions {
    open::options(2)
}

fn no_create() -> OpenOptions {
    OpenOptions {
        create: None,
        ..options()
    }
}

/// Returns relation `name` of module `app`.
fn relation(schema: &Schema, name: &str) -> RelId {
    let app = schema.module_named("app").expect("app is a module");
    match schema.module(app).values[name] {
        ValueDef::Relation(r) => r,
        other => panic!("{name} is {other:?}"),
    }
}

/// Returns a device holding `FILE` with the rows of `ROWS` and its dump at `DUMP`, and the
/// schema of `FILE`.
fn dumped() -> (Files, Schema) {
    let mut sim = Sim::new(1, Config::default());
    let schema = api::run(&mut sim, |rt| {
        let db = Database::open_in(rt, FILE, &Log::new(vec![APP.to_owned()]), &options()).unwrap();
        let schema = db.schema().clone();
        for doc in ROWS {
            let doc = read_document::<Value>(&schema, doc).unwrap();
            let rel = schema.relation(doc.relation);
            let set = Ty::Set(Box::new(Ty::Data {
                def: rel.elem_def,
                args: Vec::new(),
                ctors: None,
            }));
            let text = format!("insert app::{} from rows", rel.name);
            let rows = Value::set(doc.rows);
            db.query_text(&text, &[("rows", set, rows)], &CallOptions::default())
                .unwrap();
        }
        let dump = format!("dump to \"{DUMP}\"");
        db.query_text(&dump, &[], &CallOptions::default()).unwrap();
        db.close().unwrap();
        schema
    });
    (sim.files(), schema)
}

type Entries = BTreeMap<String, Vec<u8>>;
/// The rows of one relation on the source and on the restored file.
type Pair = (Vec<String>, Vec<String>);
type Edit = fn(&mut Entries);

/// Returns `files` with the dump at `DUMP` written again at `EDITED`, its entries edited by `edit`.
fn edited(files: &Files, edit: impl FnOnce(&mut Entries)) -> Files {
    let mut sim = Sim::with_files(2, Config::default(), files.clone());
    api::run(&mut sim, |rt| {
        let (manifest, mut entries) = storage::read_dump(&rt, DUMP, None).unwrap();
        edit(&mut entries);
        let entries: Vec<(String, Vec<u8>)> = entries.into_iter().collect();
        storage::write_dump(
            &rt,
            EDITED,
            manifest.envelope,
            manifest.snapshot,
            &entries,
            None,
        )
        .unwrap();
    });
    sim.files()
}

/// Returns the rows of `app::{name}` on `db`, printed and sorted.
fn printed(db: &Database<SimRuntime>, name: &str) -> Vec<String> {
    let text = format!("app::{name}");
    let mut results = db.query_text(&text, &[], &CallOptions::default()).unwrap();
    let rows = results.pop().unwrap();
    let form = rows.form().clone();
    let mut out: Vec<String> = rows
        .map(|r| adtb_lang::textual::print(&r.unwrap(), &form).unwrap())
        .collect();
    out.sort();
    out
}

/// Restore the dump at `from` of `files` to `COPY`. Returns the rows of each relation of `APP`
/// on `FILE` and on `COPY`.
fn restore(files: &Files, from: &str) -> Result<Vec<Pair>, Error> {
    let mut sim = Sim::with_files(3, Config::default(), files.clone());
    api::run(&mut sim, |rt| {
        let archive = ArchiveTarget::path(from).unwrap();
        restore_in(
            rt.clone(),
            &archive,
            COPY,
            RestorePoint::Latest,
            &no_create(),
        )?;
        let log = Log::new(vec![APP.to_owned()]);
        let src = Database::open_in(rt.clone(), FILE, &log, &no_create()).unwrap();
        let copy = Database::open_in(rt, COPY, &log, &no_create()).unwrap();
        let out = ["items", "refs", "odds"]
            .iter()
            .map(|r| (printed(&src, r), printed(&copy, r)))
            .collect();
        src.close().unwrap();
        copy.close().unwrap();
        Ok(out)
    })
}

fn assert_restores_the_source(files: &Files, from: &str, label: &str) {
    let rows = restore(files, from).unwrap_or_else(|e| panic!("{label}: {e:?}"));
    for (src, copy) in rows {
        assert!(!src.is_empty(), "{label}: the source holds rows");
        assert_eq!(src, copy, "{label}");
    }
}

#[test]
fn a_dump_of_every_type_restores_the_same_rows() {
    let (files, _) = dumped();
    assert_restores_the_source(&files, DUMP, "the dump");
}

#[test]
fn a_restore_reads_the_arrow_entries_and_not_the_text_entries() {
    let (files, _) = dumped();
    let text = "data/app::items.Thing.txt";
    let changed = edited(&files, |e| {
        let was = String::from_utf8(e[text].clone()).unwrap();
        let now = was.replacen("a: -128", "a: 5", 1);
        assert_ne!(was, now, "the edit applies");
        e.insert(text.to_owned(), now.into_bytes());
    });
    assert_restores_the_source(&changed, EDITED, "a text entry edited");
    let without = edited(&files, |e| e.retain(|n, _| !n.ends_with(".txt")));
    assert_restores_the_source(&without, EDITED, "no text entry");
}

#[test]
fn a_missing_damaged_cut_or_misplaced_stream_is_refused_naming_its_entry() {
    let (files, _) = dumped();
    let cases: [(&str, Edit); 4] = [
        ("missing", |e| {
            e.remove(THING);
        }),
        ("damaged", |e| e.get_mut(THING).unwrap()[0] ^= 0xff),
        ("cut", |e| {
            let b = e.get_mut(THING).unwrap();
            b.truncate(b.len() / 2);
        }),
        ("another schema", |e| {
            let rich = e["data/app::items.Rich.arrows"].clone();
            e.insert(THING.to_owned(), rich);
        }),
    ];
    for (label, edit) in cases {
        let got = restore(&edited(&files, edit), EDITED);
        assert_eq!(THING, api::damaged_dump_part(got), "{label}");
    }
}

/// Returns `v`, a value of type `t`, as the datum its Arrow form holds.
fn datum(t: &FileType, v: &Value) -> Datum {
    let all = |fs: &[FileField], xs: &[Value]| -> Vec<Datum> {
        fs.iter().zip(xs).map(|(f, x)| datum(&f.ty, x)).collect()
    };
    match (t, v) {
        (FileType::Scalar(s), v) => Datum::Scalar(scalar(*s, v)),
        (FileType::Record(fs), Value::Ctor(_, xs) | Value::Record(xs)) => {
            Datum::Record(all(fs, xs))
        }
        (FileType::Sum(_, cs), Value::Ctor(c, xs)) => Datum::Sum {
            ctor: *c,
            fields: all(&cs[*c as usize].fields, xs),
        },
        (t, v) => panic!("{v:?} is not of {t:?}"),
    }
}

fn scalar(s: Scalar, v: &Value) -> Key {
    match (s, v) {
        (Scalar::I8, Value::Int(x)) => Key::I8(i8::try_from(*x).unwrap()),
        (Scalar::I16, Value::Int(x)) => Key::I16(i16::try_from(*x).unwrap()),
        (Scalar::I32, Value::Int(x)) => Key::I32(i32::try_from(*x).unwrap()),
        (Scalar::I64, Value::Int(x)) => Key::I64(*x),
        (Scalar::F32, Value::F32(x)) => Key::F32(F32::new(x.get())),
        (Scalar::F64, Value::F64(x)) => Key::F64(F64::new(x.get())),
        (Scalar::Decimal(_), Value::Decimal(x)) => Key::Decimal(*x),
        (Scalar::Text, Value::Text(t)) => Key::Text(t.to_string()),
        (Scalar::Bytes, Value::Bytes(b)) => Key::Bytes(b.to_vec()),
        (Scalar::SortKey, Value::SortKey(b)) => Key::SortKey(b.to_vec()),
        (Scalar::Instant, Value::Instant(x)) => Key::Instant(*x),
        (Scalar::Date, Value::Date(x)) => Key::Date(*x),
        (Scalar::Time, Value::Time(x)) => Key::Time(*x),
        (Scalar::DateTime, Value::DateTime(x)) => Key::DateTime(*x),
        (Scalar::ZonedDateTime, Value::Zoned { local, zone }) => Key::ZonedDateTime {
            local: *local,
            zone: zone.to_string(),
        },
        (Scalar::Duration, Value::Duration(x)) => Key::Duration(*x),
        (Scalar::Period, Value::Period { months, days }) => Key::Period {
            months: *months,
            days: *days,
        },
        (s, v) => panic!("{v:?} is not a {s:?}"),
    }
}

fn sorted(mut rows: Vec<Vec<Datum>>) -> Vec<Vec<Datum>> {
    rows.sort_by_key(|r| format!("{r:?}"));
    rows
}

#[test]
fn arrow_rs_reads_each_data_entry_of_a_dump_as_its_constructors_rows() {
    let (files, schema) = dumped();
    let mut sim = Sim::with_files(4, Config::default(), files);
    let read = api::run(&mut sim, |rt| {
        let (_, entries) = storage::read_dump(&rt, DUMP, None).unwrap();
        let log = Log::new(vec![APP.to_owned()]);
        let db = Database::open_in(rt, FILE, &log, &no_create()).unwrap();
        let mut read = 0;
        for name in ["items", "refs", "odds"] {
            let r = relation(&schema, name);
            let text = format!("app::{name}");
            let mut results = db.query_text(&text, &[], &CallOptions::default()).unwrap();
            let rows: Vec<Value> = results.pop().unwrap().map(Result::unwrap).collect();
            let ctors = schema.ty(schema.relation(r).elem_def).ctors();
            for (c, ctor) in (0..).zip(ctors) {
                let entry = format!("data/app::{name}.{}.arrows", ctor.name);
                let file = file_schema(&schema, r, c);
                let reader = StreamReader::try_new(Cursor::new(&entries[&entry]), None).unwrap();
                assert_eq!(arrow_ipc::schema(&file), *reader.schema(), "{entry}");
                let mut got = Vec::new();
                for batch in reader {
                    let batch = batch.unwrap();
                    let columns: Vec<_> = batch.columns().iter().map(|a| a.to_data()).collect();
                    for i in 0..batch.num_rows() {
                        let fields = file.fields().iter().zip(&columns);
                        got.push(fields.map(|(f, d)| arrow_ipc::get(&f.ty, d, i)).collect());
                    }
                }
                let want = rows
                    .iter()
                    .filter(|v| matches!(v, Value::Ctor(k, _) if *k == c))
                    .map(|v| {
                        let fields = file.fields().iter().zip(v.fields().unwrap());
                        fields.map(|(f, x)| datum(&f.ty, x)).collect()
                    })
                    .collect();
                read += got.len();
                assert_eq!(sorted(want), sorted(got), "{entry}");
            }
        }
        db.close().unwrap();
        read
    });
    assert_eq!(9, read, "every row of ROWS was read");
}

#[test]
fn a_user_sum_spelled_as_bool_or_option_dumps_as_a_user_sum() {
    let (files, _) = dumped();
    let mut sim = Sim::with_files(5, Config::default(), files);
    let types = api::run(&mut sim, |rt| {
        let (_, entries) = storage::read_dump(&rt, DUMP, None).unwrap();
        let types = |entry: &str| {
            let reader = StreamReader::try_new(Cursor::new(&entries[entry]), None).unwrap();
            let fields = reader.schema().fields().clone();
            fields
                .iter()
                .map(|f| (f.name().clone(), f.data_type().clone(), f.is_nullable()))
                .collect::<Vec<_>>()
        };
        [
            types("data/app::odds.Odd.arrows"),
            types("data/app::items.Rich.arrows"),
        ]
    });
    let [odd, rich] = types;
    let dictionary = DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8));
    assert_eq!(("b".to_owned(), dictionary, false), odd[1]);
    assert!(
        matches!(&odd[2], (n, DataType::Union(_, UnionMode::Dense), false) if n == "o"),
        "{:?}",
        odd[2]
    );
    assert_eq!(("flag".to_owned(), DataType::Boolean, false), rich[1]);
    assert_eq!(("opt".to_owned(), DataType::Utf8, true), rich[2]);
}

#[test]
fn a_dump_whose_relations_span_several_record_batches_restores_the_same_rows() {
    let (files, schema) = dumped();
    let split = edited(&files, |e| {
        for name in ["items", "refs", "odds"] {
            let r = relation(&schema, name);
            let ctors = schema.ty(schema.relation(r).elem_def).ctors();
            for (c, ctor) in (0..).zip(ctors) {
                let entry = format!("data/app::{name}.{}.arrows", ctor.name);
                let rows = arrows::read_rows(&schema, r, c, &e[&entry]).unwrap();
                let stream = arrows::write_rows(&schema, r, c, &rows, 1);
                let reader = StreamReader::try_new(Cursor::new(&stream), None).unwrap();
                assert_eq!(rows.len(), reader.count(), "{entry}: one batch a row");
                e.insert(entry, stream);
            }
        }
    });
    assert_restores_the_source(&split, EDITED, "one row a batch");
}
