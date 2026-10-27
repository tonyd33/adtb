//! File scans: the rows of CSV and NDJSON files, read at a declared row type.

mod csv;
mod json;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use adtb_lang::kernel::FileFormat;
use adtb_lang::schema::{Schema, TypeDefKind};
use adtb_lang::textual::{Textual, key_field_ty};
use adtb_lang::types::{Base, CtorSet, RelId, Ty};
use unicode_normalization::UnicodeNormalization;

/// The files a file scan reads.
pub trait Files: Send + Sync {
    /// Returns the files `path` names, each by its name with its contents, in name order. A
    /// component of `path` holding `*` or `?` matches every name it matches in its directory.
    fn read(&self, path: &str) -> Result<Vec<(String, Vec<u8>)>, String>;
}

/// The files of the file system, a relative path resolved against `root`.
#[derive(Clone, Debug, Default)]
pub struct Disk {
    root: PathBuf,
}

impl Disk {
    pub fn new(root: impl Into<PathBuf>) -> Disk {
        Disk { root: root.into() }
    }
}

impl Files for Disk {
    fn read(&self, path: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
        expand(&self.root, path)?
            .into_iter()
            .map(|(name, at)| {
                fs::read(&at)
                    .map(|bytes| (name.clone(), bytes))
                    .map_err(|e| format!("{name}: {e}"))
            })
            .collect()
    }
}

/// Files held in memory, by name.
#[derive(Clone, Debug, Default)]
pub struct InMemory {
    files: BTreeMap<String, Vec<u8>>,
}

impl InMemory {
    /// Adds the file `name` holding `contents`, replacing one of that name.
    pub fn put(&mut self, name: &str, contents: impl Into<Vec<u8>>) {
        self.files.insert(name.to_string(), contents.into());
    }
}

impl Files for InMemory {
    fn read(&self, path: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
        if !is_pattern(path) {
            return match self.files.get(path) {
                Some(bytes) => Ok(vec![(path.to_string(), bytes.clone())]),
                None => Err(format!("{path}: no such file")),
            };
        }
        let parts: Vec<&str> = path.split('/').collect();
        Ok(self
            .files
            .iter()
            .filter(|(name, _)| {
                let names: Vec<&str> = name.split('/').collect();
                names.len() == parts.len()
                    && parts
                        .iter()
                        .zip(&names)
                        .all(|(p, n)| matches(p.as_bytes(), n.as_bytes()))
            })
            .map(|(name, bytes)| (name.clone(), bytes.clone()))
            .collect())
    }
}

fn is_pattern(component: &str) -> bool {
    component.contains(['*', '?'])
}

/// Returns each file `path` names under `root`, by the name `path` gives it, in name order.
fn expand(root: &Path, path: &str) -> Result<Vec<(String, PathBuf)>, String> {
    if !path.split('/').any(is_pattern) {
        return Ok(vec![(path.to_string(), root.join(path))]);
    }
    let base = if path.starts_with('/') {
        PathBuf::from("/")
    } else {
        root.to_path_buf()
    };
    let mut found: Vec<(String, PathBuf)> = vec![(String::new(), base)];
    let parts: Vec<&str> = path.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        let join = |name: &str, shown: &str| match (i, shown) {
            (0, _) => name.to_string(),
            _ => format!("{shown}/{name}"),
        };
        let mut next = Vec::new();
        for (shown, at) in found {
            if !is_pattern(part) {
                next.push((join(part, &shown), at.join(part)));
                continue;
            }
            let Ok(entries) = fs::read_dir(&at) else {
                continue;
            };
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if matches(part.as_bytes(), name.as_bytes()) {
                    next.push((join(&name, &shown), at.join(&name)));
                }
            }
        }
        found = next;
    }
    found.retain(|(_, at)| at.is_file());
    found.sort();
    Ok(found)
}

/// Returns whether `name` matches `pattern`, where `*` matches any run of bytes and `?` one.
fn matches(pattern: &[u8], name: &[u8]) -> bool {
    match (pattern.first(), name.first()) {
        (None, None) => true,
        (Some(b'*'), _) => {
            matches(&pattern[1..], name) || (!name.is_empty() && matches(pattern, &name[1..]))
        }
        (Some(b'?'), Some(_)) => matches(&pattern[1..], &name[1..]),
        (Some(p), Some(n)) if p == n => matches(&pattern[1..], &name[1..]),
        _ => false,
    }
}

/// A record as the file holds it, and its row or why it does not convert.
type Found<V> = (String, Result<V, String>);

/// One record of a file: the file, its number from 1 in the order the scan reads, its text as
/// the file holds it, and its row or why it does not convert.
#[derive(Clone, Debug)]
pub struct Record<V> {
    pub source: String,
    pub line: i64,
    pub raw: String,
    pub row: Result<V, String>,
}

/// Why a file scan read no records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The file `source` does not read as UTF-8 text in its format.
    Unreadable { source: String, detail: String },
    /// The format has no reader yet.
    NotYet(&'static str),
}

/// Returns the records of the files `path` names, in `format`, read at `row`.
pub fn records<V: Textual>(
    schema: &Schema,
    files: &dyn Files,
    format: FileFormat,
    path: &str,
    row: &Ty,
) -> Result<Vec<Record<V>>, Error> {
    let found = files.read(path).map_err(|detail| Error::Unreadable {
        source: path.to_string(),
        detail,
    })?;
    records_in(schema, format, found, row)
}

/// Returns the records of `found`, each file by its name with its contents, in `format`, read at
/// `row`, numbered from 1 across the files in their order.
pub fn records_in<V: Textual>(
    schema: &Schema,
    format: FileFormat,
    found: Vec<(String, Vec<u8>)>,
    row: &Ty,
) -> Result<Vec<Record<V>>, Error> {
    let unreadable = |source: &str, detail: String| Error::Unreadable {
        source: source.to_string(),
        detail,
    };
    let read = match format {
        FileFormat::Csv => csv::records::<V>,
        FileFormat::Ndjson => json::records::<V>,
        FileFormat::Parquet => return Err(Error::NotYet("a Parquet file scan")),
        FileFormat::Arrow => return Err(Error::NotYet("an Arrow IPC file scan")),
    };
    let mut out = Vec::new();
    for (source, bytes) in found {
        let text = String::from_utf8(bytes)
            .map_err(|e| unreadable(&source, format!("the file is not UTF-8: {e}")))?;
        let found = read(schema, &text, row).map_err(|e| unreadable(&source, e))?;
        for (raw, row) in found {
            out.push(Record {
                source: source.clone(),
                line: out.len() as i64 + 1,
                raw,
                row,
            });
        }
    }
    Ok(out)
}

/// How a value of a type is read from a flat format.
enum Shape {
    /// A scalar, read from its textual syntax.
    Scalar,
    Bool,
    Text,
    Option(Ty),
    /// A sum whose constructors have no fields, read from a constructor name; the constructors
    /// the type allows when it restricts them.
    Names(Vec<String>, Option<CtorSet>),
    Key(RelId),
    Newtype(Ty),
    /// A record's fields, and whether it is a declared record rather than an anonymous one.
    Record(Vec<(String, Ty)>, bool),
    Sum(Sum),
    /// A type with no flat form.
    Other,
}

struct Sum {
    header: Vec<(String, Ty)>,
    ctors: Vec<(String, Vec<(String, Ty)>)>,
    allowed: Option<CtorSet>,
}

impl Sum {
    /// Returns the index of the constructor `name`, which the type must allow.
    fn ctor(&self, name: &str) -> Result<u32, String> {
        let i = self
            .ctors
            .iter()
            .position(|(n, _)| n == name)
            .ok_or_else(|| format!("`{name}` is not a constructor of the type"))?;
        allowed(i as u32, name, self.allowed.as_ref())
    }
}

fn allowed(i: u32, name: &str, set: Option<&CtorSet>) -> Result<u32, String> {
    match set {
        Some(s) if !s.contains(&i) => {
            Err(format!("constructor `{name}` is not one the type allows"))
        }
        _ => Ok(i),
    }
}

fn shape(schema: &Schema, ty: &Ty) -> Shape {
    match ty {
        Ty::Base(Base::Text) => Shape::Text,
        Ty::Base(_) | Ty::Unit => Shape::Scalar,
        Ty::Key { rel, .. } => Shape::Key(*rel),
        Ty::Record(fs) => Shape::Record(fs.clone(), false),
        Ty::Tag(inner) => {
            let names = match inner.as_ref() {
                Ty::Data { def, .. } => schema
                    .ty(*def)
                    .ctors()
                    .iter()
                    .map(|c| c.name.clone())
                    .collect(),
                Ty::Sum { ctors, .. } => ctors.iter().map(|(n, _)| n.clone()).collect(),
                _ => return Shape::Other,
            };
            Shape::Names(names, None)
        }
        Ty::Sum { ctors, sel } => sum_shape(Vec::new(), ctors.clone(), sel.clone()),
        Ty::Data { def, args, ctors } => {
            let d = schema.ty(*def);
            let prelude = |name: &str| *def == schema.prelude_type(name);
            match &d.kind {
                TypeDefKind::Newtype { .. } if prelude("Uuid") => Shape::Scalar,
                TypeDefKind::Newtype { base, .. } => Shape::Newtype(base.subst(args)),
                TypeDefKind::Sum { .. } if prelude("Span") => Shape::Scalar,
                TypeDefKind::Sum { .. } if prelude("Bool") => Shape::Bool,
                TypeDefKind::Sum { .. } if prelude("Option") => Shape::Option(args[0].clone()),
                TypeDefKind::Sum {
                    header,
                    ctors: all,
                    record,
                } => {
                    let fields = |fs: &[adtb_lang::schema::FieldDef]| -> Vec<(String, Ty)> {
                        fs.iter()
                            .map(|f| (f.name.clone(), f.ty.subst(args)))
                            .collect()
                    };
                    if *record {
                        let mut fs = fields(header);
                        fs.extend(fields(&all[0].fields));
                        return Shape::Record(fs, true);
                    }
                    sum_shape(
                        fields(header),
                        all.iter()
                            .map(|c| (c.name.clone(), fields(&c.fields)))
                            .collect(),
                        ctors.clone(),
                    )
                }
            }
        }
        _ => Shape::Other,
    }
}

fn sum_shape(
    header: Vec<(String, Ty)>,
    ctors: Vec<(String, Vec<(String, Ty)>)>,
    allowed: Option<CtorSet>,
) -> Shape {
    if header.is_empty() && ctors.iter().all(|(_, fs)| fs.is_empty()) {
        return Shape::Names(ctors.into_iter().map(|(n, _)| n).collect(), allowed);
    }
    Shape::Sum(Sum {
        header,
        ctors,
        allowed,
    })
}

/// Returns the type of the value of a key of `rel`: its one key field's, or the record of its
/// key fields.
fn key_value_ty(schema: &Schema, rel: RelId) -> Ty {
    let r = schema.relation(rel);
    match r.key.as_slice() {
        [f] => key_field_ty(schema, rel, f),
        fs => {
            let mut fields: Vec<(String, Ty)> = fs
                .iter()
                .map(|f| (f.clone(), key_field_ty(schema, rel, f)))
                .collect();
            fields.sort_by(|a, b| a.0.cmp(&b.0));
            Ty::Record(fields)
        }
    }
}

/// Returns the value of a record or constructor from its fields in declaration order.
fn record<V: Textual>(fields: Vec<(String, V)>, declared: bool) -> V {
    let (names, values): (Vec<String>, Vec<V>) = fields.into_iter().unzip();
    let fields = names.iter().map(String::as_str).zip(values).collect();
    if declared {
        V::of_ctor(0, fields)
    } else {
        V::of_record(fields)
    }
}

/// Returns the value of constructor `c` from its fields, header fields first, in declaration
/// order.
fn ctor<V: Textual>(c: u32, fields: Vec<(String, V)>) -> V {
    let (names, values): (Vec<String>, Vec<V>) = fields.into_iter().unzip();
    V::of_ctor(c, names.iter().map(String::as_str).zip(values).collect())
}

fn text<V: Textual>(t: &str) -> V {
    let t: String = t.nfc().collect();
    V::of_scalar(adtb_lang::textual::Scalar::Text(&t), Base::Text)
}

fn none<V: Textual>() -> V {
    V::of_ctor(0, Vec::new())
}

fn some<V: Textual>(v: V) -> V {
    V::of_ctor(1, vec![("value", v)])
}

/// Returns `text` read in the textual syntax at `ty`.
fn parse<V: Textual>(schema: &Schema, text: &str, ty: &Ty) -> Result<V, String> {
    adtb_lang::textual::parse(text, &adtb_lang::textual::Form::of(schema, ty))
        .map_err(|e| e.to_string())
}
