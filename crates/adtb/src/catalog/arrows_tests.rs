use adtb_lang::evolve::Source;
use adtb_lang::parse::{Options, parse_migration};
use adtb_lang::schema::{Schema, ValueDef};

use crate::batch::ipc::{self, Budget, FileField, FileSchema, FileType, IpcError};
use crate::batch::{Batch, Schema as Columns};
use crate::key::Value;

use super::super::layout::{batch_type, row};
use super::{file_schema, row_budget};

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
";
const ODD: &str = "module odd
add module odd
add type Flag = False | True
add type Maybe = None | Some { value: I64 }
add type Odd = { id: I64, b: Flag, o: Maybe }
add relation odds : Odd key id
";

fn schema() -> Schema {
    let mut source = Source::default();
    for text in [APP, ODD] {
        let file = parse_migration(text, Options::default())
            .ok()
            .expect("the migration parses");
        source = source
            .migrate(&file, text)
            .unwrap_or_else(|r| panic!("the migration applies: {r:?}"));
    }
    source.schema().expect("it builds")
}

fn shape(t: &FileType) -> String {
    match t {
        FileType::Scalar(s) => format!("{s:?}"),
        FileType::Record(fs) => format!("{{{}}}", shapes(fs)),
        FileType::Sum(kind, cs) => {
            let cs: Vec<String> = cs
                .iter()
                .map(|c| format!("{}({})", c.name, shapes(&c.fields)))
                .collect();
            format!("{kind:?}[{}]", cs.join(" | "))
        }
        FileType::List(e) => format!("[{}]", shape(e)),
    }
}

fn shapes(fs: &[FileField]) -> String {
    let fs: Vec<String> = fs
        .iter()
        .map(|f| format!("{}: {}", f.name, shape(&f.ty)))
        .collect();
    fs.join(", ")
}

#[test]
fn a_rows_columns_name_every_field_and_constructor_mark_bool_and_option_by_identity_and_count_its_bits()
 {
    let schema = schema();
    let rel = |module: &str, name: &str| {
        let m = schema.module_named(module).expect("the module exists");
        match schema.module(m).values[name] {
            ValueDef::Relation(r) => r,
            other => panic!("{name} is {other:?}"),
        }
    };
    let via = "Other[Email(addr: Text) | Phone(num: Text)]";
    let cases = [
        (
            rel("app", "items"),
            0,
            856,
            "id: I64, a: I8, b: I16, c: I32, d: I64, e: F32, f: F64, g: Decimal(4), h: Text, \
             i: Bytes, j: SortKey, k: Instant, l: Date, m: Time, n: DateTime, o: ZonedDateTime, \
             p: Duration, q: Period"
                .to_owned(),
        ),
        (
            rel("app", "items"),
            1,
            337,
            format!(
                "id: I64, flag: Bool[False() | True()], \
                 opt: Option[None() | Some(value: Text)], \
                 opts: Option[None() | Some(value: Option[None() | Some(value: I64)])], \
                 optv: Option[None() | Some(value: {via})], \
                 color: Other[Red() | Green() | Blue()], via: {via}, wrap: Other[Wrap(n: I32)], \
                 name: Text, at: {{x: I16, y: I8}}"
            ),
        ),
        (rel("app", "items"), 2, 64, "id: I64".to_owned()),
        (rel("app", "refs"), 0, 128, "id: I64, to: I64".to_owned()),
        (
            rel("odd", "odds"),
            0,
            136,
            "id: I64, b: Other[False() | True()], o: Other[None() | Some(value: I64)]".to_owned(),
        ),
    ];
    for (r, ctor, bits, expected) in cases {
        let file = file_schema(&schema, r, ctor);
        assert_eq!(expected, shapes(file.fields()), "{r:?} {ctor}");
        assert_eq!(bits, file.row_bits(), "{r:?} {ctor}");
        let columns = row(&schema, r, ctor)
            .iter()
            .map(|(_, t)| batch_type(&schema, t).expect("a row's field has a column"))
            .collect();
        assert_eq!(&Columns::new(columns), file.schema(), "{r:?} {ctor}");
    }
}

#[test]
fn a_stream_of_more_than_one_row_of_no_bits_is_refused() {
    let file = FileSchema::new(vec![FileField {
        name: "u".to_owned(),
        ty: FileType::Record(Vec::new()),
    }]);
    assert_eq!(0, file.row_bits());
    let rows = vec![[Value::Record(Vec::new())]; 2];
    let batch = Batch::from_rows(file.schema().clone(), rows.iter().map(|r| &r[..]));
    let mut bytes = Vec::new();
    ipc::write(&file, [&batch], &mut bytes).unwrap();
    assert!(bytes.len() > 2, "a byte a row would admit both rows");
    let budget = row_budget(&file, bytes.len());
    let got = ipc::read(&bytes, &file, &mut Budget::new(budget));
    assert_eq!(Err(IpcError::OverBudget("rows")), got.map(drop));
}
