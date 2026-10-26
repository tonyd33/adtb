use std::collections::BTreeSet;

use super::read::{ParseError, Reader};
use super::{Form, Textual, print};
use crate::schema::Schema;
use crate::types::RelId;

/// The first line of a versioned document: the edition of the textual syntax and the schema
/// version, with its hash, that its names are written at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub edition: String,
    pub version: u64,
    pub hash: String,
}

/// A text document holding rows of one relation variable. A document with no header is written
/// at the reader's version.
#[derive(Clone, Debug, PartialEq)]
pub struct Document<V> {
    pub header: Option<Header>,
    pub relation: RelId,
    pub rows: BTreeSet<V>,
}

impl Reader<'_> {
    fn header(&mut self) -> Result<Header, ParseError> {
        self.ws();
        let start = self.pos;
        let year = self.some_digits()?;
        if !self.src[self.pos..].starts_with('-') {
            return self.fail("expected an edition, `YYYY-MM`");
        }
        self.pos += 1;
        let month = self.some_digits()?;
        if year.len() != 4 || month.len() != 2 {
            self.pos = start;
            return self.fail("expected an edition, `YYYY-MM`");
        }
        let edition = self.src[start..self.pos].to_string();
        if self.ident()? != "pins" {
            return self.fail("expected `pins`");
        }
        self.ws();
        let at = self.pos;
        let version = self.some_digits()?.parse().or_else(|_| {
            self.pos = at;
            self.fail("a version out of range")
        })?;
        self.expect("(")?;
        let at = self.pos;
        while self.src[self.pos..].starts_with(|c: char| c.is_ascii_hexdigit()) {
            self.pos += 1;
        }
        let hash = self.src[at..self.pos].to_string();
        if hash.len() != 64 {
            self.pos = at;
            return self.fail("expected a hash of 64 hex digits");
        }
        self.expect(")")?;
        Ok(Header {
            edition,
            version,
            hash,
        })
    }
}

/// Reads a document: an optional header line `text EDITION pins VERSION (HASH)`, then
/// `data RELATION`, then the rows of the relation in the textual syntax, in any order.
pub fn read_document<V: Textual>(schema: &Schema, text: &str) -> Result<Document<V>, ParseError> {
    let mut r = Reader::new(text);
    r.ws();
    let at = r.pos;
    let mut header = None;
    if r.ident()? == "text" {
        header = Some(r.header()?);
    } else {
        r.pos = at;
    }
    if r.ident()? != "data" {
        return r.fail("expected `data`");
    }
    let at = r.pos;
    let first = r.ident()?;
    let (module, name) = if r.src[r.pos..].starts_with("::") {
        r.pos += 2;
        (Some(first), r.ident()?)
    } else {
        (None, first)
    };
    let Some(i) = schema.relations.iter().position(|d| {
        d.name == name && module.is_none_or(|m| schema.modules[d.module.0 as usize].name == m)
    }) else {
        r.pos = at;
        return r.fail(format!("no relation `{name}`"));
    };
    let rel = RelId(i as u32);
    let form = Form::of(schema, &schema.relation(rel).elem);
    let mut rows = BTreeSet::new();
    while !r.at_end() {
        rows.insert(r.value(&form)?);
    }
    Ok(Document {
        header,
        relation: rel,
        rows,
    })
}

/// Returns `doc` as text: its header, its `data` line, and one row per line in ascending order.
pub fn print_document<V: Textual>(schema: &Schema, doc: &Document<V>) -> String {
    let r = schema.relation(doc.relation);
    let form = Form::of(schema, &r.elem);
    let mut out = String::new();
    if let Some(h) = &doc.header {
        out.push_str(&format!(
            "text {} pins {} ({})\n",
            h.edition, h.version, h.hash
        ));
    }
    out.push_str(&format!("data {}\n", r.name));
    for row in &doc.rows {
        out.push_str(&print(row, &form));
        out.push('\n');
    }
    out
}
