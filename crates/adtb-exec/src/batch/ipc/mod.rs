//! Write batches as an Arrow IPC stream in the external type mapping, and read such a stream
//! back.

mod flat;
mod read;
mod schema;
mod write;

use std::fmt;

use adtb_lang::textual::{self, Form as Text};
use adtb_lang::types::Base;

use super::types::union_group;
use super::{Schema, Type};
use crate::exec::Val;
use crate::key::{CtorId, Scalar, Value};

pub use read::read;
pub use write::write;

/// A column or field: its name and type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileField {
    pub name: String,
    pub ty: FileType,
}

/// A batch [`Type`] with the names and identities its Arrow mapping needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileType {
    Scalar(Scalar),
    Record(Vec<FileField>),
    Sum(SumKind, Vec<FileCtor>),
    List(Box<FileType>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileCtor {
    pub id: CtorId,
    pub name: String,
    pub fields: Vec<FileField>,
}

/// Which sum a [`FileType::Sum`] is.
///
/// Preconditions:
/// - `Bool`'s constructors are `False` then `True`, both nullary.
/// - `Option`'s are `None`, nullary, then `Some` with one field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SumKind {
    Bool,
    Option,
    Other,
}

/// The columns of a stream and the batch [`Schema`] they hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileSchema {
    fields: Vec<FileField>,
    schema: Schema,
}

impl FileSchema {
    pub fn new(fields: Vec<FileField>) -> FileSchema {
        let schema = Schema::new(fields.iter().map(|f| batch_type(&f.ty)).collect());
        FileSchema { fields, schema }
    }

    pub fn fields(&self) -> &[FileField] {
        &self.fields
    }

    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Returns the fewest bits of a stream's body one row of these columns takes.
    pub fn row_bits(&self) -> usize {
        self.fields.iter().map(|f| bits(form(&f.ty))).sum()
    }
}

fn bits(f: Form<'_>) -> usize {
    match f {
        Form::Scalar(
            Scalar::Text | Scalar::Bytes | Scalar::SortKey | Scalar::Period | Scalar::ZonedDateTime,
        )
        | Form::Enum(_)
        | Form::List(_) => 32,
        Form::Scalar(s) => 8 * width(s),
        Form::Bool => 1,
        Form::Struct { fields, .. } => fields.iter().map(|f| bits(form(&f.ty))).sum(),
        Form::Union { .. } => 40,
        Form::Nullable(t) => bits(form(t)),
    }
}

fn batch_type(t: &FileType) -> Type {
    match t {
        FileType::Scalar(s) => Type::Scalar(*s),
        FileType::Record(fs) => Type::Record(fs.iter().map(|f| batch_type(&f.ty)).collect()),
        FileType::Sum(_, cs) => Type::Sum(
            cs.iter()
                .map(|c| super::Ctor {
                    id: c.id,
                    fields: c.fields.iter().map(|f| batch_type(&f.ty)).collect(),
                })
                .collect(),
        ),
        FileType::List(e) => Type::List(Box::new(batch_type(e))),
    }
}

/// Why a stream does not read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IpcError {
    /// The bytes are not a well-formed Arrow IPC stream.
    Malformed(&'static str),
    /// The stream's field at `path` differs from the expected one in `what`.
    SchemaDiffers { path: String, what: &'static str },
    /// A value of column `column`, in row `row` when the reader knows it, does not convert to its
    /// field's type or lies outside its domain.
    BadValue {
        column: String,
        row: Option<usize>,
        what: &'static str,
    },
    /// The stream holds more than its [`Budget`] allows of `what`.
    OverBudget(&'static str),
}

/// What one [`read`] may materialise.
#[derive(Clone, Debug)]
pub struct Budget {
    rows: usize,
    datums: usize,
    bytes: usize,
}

impl Budget {
    /// Returns a budget of `rows` rows: it bounds the record batches' total length to `rows`,
    /// and the datums a read materialises to `rows` times the most one row of the schema costs,
    /// each element of a list counted as a row. Text, binary values and dictionary entries are
    /// bounded by the stream's length.
    pub fn new(rows: usize) -> Budget {
        Budget {
            rows,
            datums: 0,
            bytes: 0,
        }
    }

    /// Returns how many datums the last read materialised.
    pub fn datums(&self) -> usize {
        self.datums
    }

    /// Returns how many value bytes and dictionary entries the last read charged.
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

fn bad_value(what: &'static str) -> IpcError {
    IpcError::BadValue {
        column: String::new(),
        row: None,
        what,
    }
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IpcError::Malformed(what) => write!(f, "malformed Arrow IPC stream: {what}"),
            IpcError::SchemaDiffers { path, what } => {
                write!(f, "the stream's field {path} differs in {what}")
            }
            IpcError::BadValue { column, row, what } => {
                write!(
                    f,
                    "a value of column {column} at row {row:?} does not convert: {what}"
                )
            }
            IpcError::OverBudget(what) => write!(f, "the stream holds more {what} than allowed"),
        }
    }
}

impl std::error::Error for IpcError {}

/// The Arrow layout of a [`FileType`].
#[derive(Clone, Copy, Debug)]
enum Form<'a> {
    Scalar(Scalar),
    Bool,
    Enum(&'a [FileCtor]),
    /// A struct of `fields`, holding a record or, when `sum`, a one-constructor sum.
    Struct {
        fields: &'a [FileField],
        sum: bool,
    },
    List(&'a FileType),
    /// A dense union over constructors from position `base` on.
    Union {
        ctors: &'a [FileCtor],
        base: usize,
    },
    /// The present column of an `Option`, nullable.
    Nullable(&'a FileType),
}

fn form(t: &FileType) -> Form<'_> {
    match t {
        FileType::Scalar(s) => Form::Scalar(*s),
        FileType::Record(fields) => Form::Struct { fields, sum: false },
        FileType::List(e) => Form::List(e),
        FileType::Sum(_, cs) if cs.len() == 1 => Form::Struct {
            fields: &cs[0].fields,
            sum: true,
        },
        FileType::Sum(SumKind::Bool, _) => Form::Bool,
        FileType::Sum(_, cs) if cs.iter().all(|c| c.fields.is_empty()) => Form::Enum(cs),
        FileType::Sum(SumKind::Option, cs) if takes_validity(&cs[1].fields[0].ty) => {
            Form::Nullable(&cs[1].fields[0].ty)
        }
        FileType::Sum(_, ctors) => Form::Union { ctors, base: 0 },
    }
}

fn takes_validity(t: &FileType) -> bool {
    !matches!(t, FileType::Sum(SumKind::Option, _)) && !matches!(form(t), Form::Union { .. })
}

/// Returns the children of a union over `ctors` from position `base` on, each with its name: a
/// constructor's struct, or past [`super::UNION_LIMIT`] constructors a union over a group.
fn union_kids(ctors: &[FileCtor], base: usize) -> Vec<(String, Form<'_>)> {
    let g = union_group(ctors.len());
    if g == 1 {
        return ctors
            .iter()
            .map(|c| {
                (
                    c.name.clone(),
                    Form::Struct {
                        fields: &c.fields,
                        sum: true,
                    },
                )
            })
            .collect();
    }
    ctors
        .chunks(g)
        .enumerate()
        .map(|(j, ctors)| {
            (
                j.to_string(),
                Form::Union {
                    ctors,
                    base: base + j * g,
                },
            )
        })
        .collect()
}

/// Returns the width of a fixed-width scalar's Arrow values.
fn width(s: Scalar) -> usize {
    match s {
        Scalar::I8 => 1,
        Scalar::I16 => 2,
        Scalar::I32 | Scalar::F32 | Scalar::Date => 4,
        Scalar::Decimal(_) => 16,
        _ => 8,
    }
}

fn text_form(s: Scalar) -> Text {
    Text::Base(match s {
        Scalar::Period => Base::Period,
        _ => Base::ZonedDateTime,
    })
}

/// Returns the textual syntax of a `Period` or `ZonedDateTime`.
fn to_text(v: &Value, s: Scalar) -> String {
    let v = match v {
        Value::Period { months, days } => Val::Period {
            months: *months,
            days: *days,
        },
        Value::ZonedDateTime { local, zone } => Val::Zoned {
            local: *local,
            zone: zone.as_str().into(),
        },
        v => panic!("value {v:?} is not a Period or ZonedDateTime"),
    };
    textual::print(&v, &text_form(s)).expect("a Period or ZonedDateTime has a textual syntax")
}

fn of_text(text: &str, s: Scalar) -> Result<Value, IpcError> {
    match textual::parse::<Val>(text, &text_form(s)) {
        Ok(Val::Period { months, days }) => Ok(Value::Period { months, days }),
        Ok(Val::Zoned { local, zone }) => Ok(Value::ZonedDateTime {
            local,
            zone: zone.to_string(),
        }),
        _ => Err(bad_value("text not in the type's textual syntax")),
    }
}
