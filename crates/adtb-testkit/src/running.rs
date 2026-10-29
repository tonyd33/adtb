//! The running schema of the spec at each version, and as each part extends it.

use std::collections::BTreeMap;

use adtb_lang::ast::{CtorSel, Decl, ExampleItem, Expr, ExprKind, FnBody, Operator, OperatorKind};
use adtb_lang::lex::{Tok, lex};
use adtb_lang::parse::{Options, parse_example};
use adtb_lang::span::Span;
use adtb_lang::visit::each_expr;

use crate::spec::Block;

/// The block that declares the schema at version 10, where the history starts.
const VERSION_10: (&str, &str) = ("README.md", "set OAuth of identities = Google\n");

/// The block that declares the running schema, at version 12.
const VERSION_12: (&str, &str) = ("README.md", "set OAuth of identities = Google | Apple");

/// The block of evolution.md that declares version 10.
const EVOLUTION_10: (&str, &str) = ("evolution.md", "constraint login_present =");

/// The migrations from version 10 to version 12.
const MIGRATIONS: &[(&str, &str)] = &[
    ("evolution.md", "-- 0011, from version 10"),
    ("evolution.md", "-- 0012, continuing"),
];

/// The blocks each part extends version 12 with, in order.
const EXTENSIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "applications.md",
        &[("applications.md", "type Session { id: Key sessions }")],
    ),
    (
        "constraints.md",
        &[
            ("constraints.md", "type Entry = { id: Key entries"),
            ("constraints.md", "add index (user) in identities"),
        ],
    ),
    (
        "derived.md",
        &[
            ("applications.md", "type Session { id: Key sessions }"),
            ("derived.md", "module billing\ntype Invoice"),
            ("derived.md", "-- 0030, from version 29"),
            ("derived.md", "add type Payment"),
            ("derived.md", "type Shape { id: Key shapes }"),
        ],
    ),
    (
        "evolution.md",
        &[("evolution.md", "type Badge { name: Text }")],
    ),
    (
        "interop.md",
        &[("interop.md", "-- 0019\nmodule auth\ndefine fn geohash")],
    ),
    ("model.md", &[("model.md", "role: Text }\ntype Visit")]),
    ("queries.md", &[("queries.md", "type Post = {")]),
    (
        "security.md",
        &[
            ("security.md", "type Invoice { id: Key invoices, owner"),
            (
                "security.md",
                "type Principal = EndUser { user: Key users }",
            ),
        ],
    ),
    (
        "syntax.md",
        &[
            ("queries.md", "type Post = {"),
            ("derived.md", "module billing\ntype Invoice"),
            ("derived.md", "-- 0030, from version 29"),
            ("syntax.md", "type Import { id: Key imports }"),
        ],
    ),
    (
        "writes.md",
        &[
            ("writes.md", "fn rehash (h: Bytes)"),
            ("writes.md", "add unique (lower(email))"),
        ],
    ),
];

/// One declaration of a schema, as source text.
#[derive(Clone, Debug)]
pub struct Entry {
    pub module: String,
    pub key: (&'static str, String),
    pub text: String,
    /// The spec file and line the text starts at.
    pub file: String,
    pub line: usize,
}

impl Entry {
    /// Returns whether `text` spells this declaration, token for token.
    pub fn spelled(&self, text: &str) -> bool {
        tokens(&self.text) == tokens(text)
    }
}

pub(crate) fn tokens(src: &str) -> Vec<&str> {
    lex(src)
        .0
        .into_iter()
        .filter(|t| t.tok != Tok::Eof)
        .map(|t| &src[t.span.range()])
        .collect()
}

pub fn decl_key(d: &Decl) -> (&'static str, String) {
    match d {
        Decl::Type(t) => ("type", t.name.text.clone()),
        Decl::Relation(r) => ("relation", r.name.name.text.clone()),
        Decl::Set(s) => ("set", format!("{}.{}", s.relation.text, s.name.text)),
        Decl::Fn(f) => ("fn", f.name.text.clone()),
        Decl::Constraint(c) => ("constraint", c.name.text.clone()),
        Decl::Check(c) => ("check", c.name.text.clone()),
        Decl::Store(s) => ("store", s.view.name.text.clone()),
        Decl::Aggregate(a) => ("aggregate", a.name.text.clone()),
    }
}

pub fn decl_span(d: &Decl) -> Span {
    match d {
        Decl::Type(t) => t.span,
        Decl::Relation(r) => r.span,
        Decl::Set(s) => s.span,
        Decl::Fn(f) => f.span,
        Decl::Constraint(c) => c.span,
        Decl::Check(c) => c.span,
        Decl::Store(s) => s.span,
        Decl::Aggregate(a) => a.span,
    }
}

/// The line of the spec file that byte `at` of the block's text is on.
pub(crate) fn spec_line(b: &Block, at: usize) -> usize {
    b.line + 1 + b.text[..at].matches('\n').count()
}

fn find<'a>(blocks: &'a [Block], (file, needle): (&str, &str)) -> &'a Block {
    let found: Vec<&Block> = blocks
        .iter()
        .filter(|b| b.file == file && b.text.contains(needle))
        .collect();
    assert!(!found.is_empty(), "{file}: no block contains {needle:?}");
    assert!(
        found.iter().all(|b| b.text == found[0].text),
        "{file}: blocks containing {needle:?} differ"
    );
    found[0]
}

fn items(b: &Block) -> Vec<ExampleItem> {
    parse_example(&b.text, Options { elisions: true })
        .ok()
        .unwrap_or_else(|_| panic!("{}:{} does not parse", b.file, b.line))
}

/// Adds the declarations of `b` to `schema` and applies its operators.
fn declare(schema: &mut Vec<Entry>, b: &Block) {
    let mut module = "auth".to_string();
    for item in items(b) {
        match item {
            ExampleItem::Module(m) => module = m.text,
            ExampleItem::Decl(d) => push(schema, &module, b, &d),
            ExampleItem::Operator(op) => apply(schema, &module, b, &op),
            _ => panic!(
                "{}:{} declares something other than a schema",
                b.file, b.line
            ),
        }
    }
}

pub(crate) fn push(schema: &mut Vec<Entry>, module: &str, b: &Block, d: &Decl) {
    let key = decl_key(d);
    let span = decl_span(d);
    let text = b.text[span.range()].to_string();
    if schema
        .iter()
        .any(|e| e.module == module && e.key == key && e.spelled(&text))
    {
        return;
    }
    schema.push(Entry {
        module: module.to_string(),
        key,
        text,
        file: b.file.clone(),
        line: spec_line(b, span.start as usize),
    });
}

fn entry<'a>(schema: &'a mut [Entry], module: &str, kind: &str, name: &str) -> &'a mut Entry {
    schema
        .iter_mut()
        .find(|e| e.module == module && e.key.0 == kind && e.key.1 == name)
        .unwrap_or_else(|| panic!("the running schema has no {kind} `{name}`"))
}

/// The text of `b` from the start of `from` to the end of `to`.
fn slice(b: &Block, from: Span, to: Span) -> &str {
    &b.text[from.start as usize..to.end as usize]
}

fn sel_text(sel: &CtorSel) -> String {
    match sel {
        CtorSel::One(c) => c.text.clone(),
        CtorSel::Many(cs) => {
            let names: Vec<&str> = cs.iter().map(|c| c.text.as_str()).collect();
            format!("({})", names.join(" | "))
        }
    }
}

/// The end of the record that opens at or after `at`, or `at` when none does.
fn record_end(src: &str, at: usize) -> usize {
    let rest = &src[at..];
    let Some(open) = rest.find(|c: char| !c.is_whitespace()) else {
        return at;
    };
    if !rest[open..].starts_with('{') {
        return at;
    }
    let mut depth = 0;
    for (i, c) in rest[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return at + open + i + 1;
                }
            }
            _ => {}
        }
    }
    panic!("unclosed record")
}

/// The one `match` of a function or constraint body, as a span of its text.
fn the_match(e: &Entry) -> Span {
    let src = format!("module {}\n{}", e.module, e.text);
    let prefix = src.len() - e.text.len();
    let items = parse_example(&src, Options { elisions: true })
        .ok()
        .expect("parses");
    let body: Expr = match items.into_iter().nth(1) {
        Some(ExampleItem::Decl(Decl::Fn(f))) => match f.body {
            FnBody::Expr(x) => x,
            _ => panic!("`{}` has no body", e.key.1),
        },
        Some(ExampleItem::Decl(Decl::Constraint(c))) => c.query,
        _ => panic!("`{}` is neither a function nor a constraint", e.key.1),
    };
    let mut found = Vec::new();
    each_expr(&body, &mut |x| {
        if matches!(x.kind, ExprKind::Match(..)) {
            found.push(x.span);
        }
    });
    assert_eq!(found.len(), 1, "`{}` has one match", e.key.1);
    let s = found[0];
    Span::new(s.start as usize - prefix, s.end as usize - prefix)
}

fn append(e: &mut Entry, text: &str) {
    e.text.push_str(text);
}

/// Applies a migration operator to the declarations it changes.
fn apply(schema: &mut Vec<Entry>, module: &str, b: &Block, op: &Operator) {
    match &op.kind {
        OperatorKind::AddConstructor {
            path,
            record,
            place: None,
            joins,
            arms,
        } => {
            let start = path.ctor.span.start as usize;
            let end = match record {
                Some(_) => record_end(&b.text, path.ctor.span.end as usize),
                None => path.ctor.span.end as usize,
            };
            let ctor = &b.text[start..end];
            append(
                entry(schema, module, "type", &path.ty.name.text),
                &format!("\n  | {ctor}"),
            );
            for j in joins {
                let Some(CtorSel::One(set)) = &j.sel else {
                    panic!("a constructor joins a named set");
                };
                let name = format!("{}.{}", j.relation.name.text, set.text);
                append(
                    entry(schema, module, "set", &name),
                    &format!(" | {}", path.ctor.text),
                );
            }
            for a in arms {
                assert!(a.field.is_none(), "an arm of a computed field");
                let arm = slice(b, a.pattern.span, a.body.span).to_string();
                let target = &a.target.name.text;
                let kind = if schema
                    .iter()
                    .any(|e| e.module == module && e.key == ("fn", target.clone()))
                {
                    "fn"
                } else {
                    "constraint"
                };
                let e = entry(schema, module, kind, target);
                let at = the_match(e).end as usize;
                e.text.insert_str(at, &format!("\n  | {arm}"));
            }
        }
        OperatorKind::AddUnique(fields, on) => {
            let fields = slice(b, fields[0].span, fields[fields.len() - 1].span);
            let scope = on
                .sel
                .as_ref()
                .map_or(String::new(), |s| format!(" in {}", sel_text(s)));
            append(
                entry(schema, module, "relation", &on.relation.name.text),
                &format!("\n  unique ({fields}){scope}"),
            );
        }
        OperatorKind::AddIndex {
            interval,
            fields,
            on,
        } => {
            let fields = slice(b, fields[0].span, fields[fields.len() - 1].span);
            let scope = on
                .sel
                .as_ref()
                .map_or(String::new(), |s| format!(" in {}", sel_text(s)));
            let kind = if *interval { "interval index" } else { "index" };
            append(
                entry(schema, module, "relation", &on.relation.name.text),
                &format!("\n  {kind} ({fields}){scope}"),
            );
        }
        OperatorKind::AddType(t) => push(schema, module, b, &Decl::Type(t.clone())),
        OperatorKind::AddRelation(r) => push(schema, module, b, &Decl::Relation(r.clone())),
        OperatorKind::DefineFn(f) => push(schema, module, b, &Decl::Fn(f.clone())),
        OperatorKind::Enable(_) => {}
        other => panic!("the running schema's history applies {other:?}"),
    }
}

/// The running schema, at version 12.
pub fn version_12(blocks: &[Block]) -> Vec<Entry> {
    let mut schema = Vec::new();
    declare(&mut schema, find(blocks, VERSION_12));
    schema
}

/// The schema at version 10, as README.md and evolution.md declare it.
pub fn version_10(blocks: &[Block]) -> (Vec<Entry>, Vec<Entry>) {
    let mut readme = Vec::new();
    declare(&mut readme, find(blocks, VERSION_10));
    let mut evolution = Vec::new();
    declare(&mut evolution, find(blocks, EVOLUTION_10));
    (readme, evolution)
}

/// The schema at version 10 after migrations 0011 and 0012.
pub fn folded_12(blocks: &[Block]) -> Vec<Entry> {
    let mut schema = Vec::new();
    declare(&mut schema, find(blocks, VERSION_10));
    for &m in MIGRATIONS {
        declare(&mut schema, find(blocks, m));
    }
    schema
}

/// The declarations of `a` that `b` lacks or spells otherwise, and those of `b` that `a` lacks.
pub fn differences(a: &[Entry], b: &[Entry]) -> Vec<String> {
    let mut out = Vec::new();
    for x in a {
        match b.iter().find(|y| y.module == x.module && y.key == x.key) {
            None => out.push(format!(
                "{}:{}: only on the left: {} `{}`",
                x.file, x.line, x.key.0, x.key.1
            )),
            Some(y) if !y.spelled(&x.text) => out.push(format!(
                "{}:{} and {}:{}: {} `{}` differs",
                x.file, x.line, y.file, y.line, x.key.0, x.key.1
            )),
            Some(_) => {}
        }
    }
    for y in b {
        if !a.iter().any(|x| x.module == y.module && x.key == y.key) {
            out.push(format!(
                "{}:{}: only on the right: {} `{}`",
                y.file, y.line, y.key.0, y.key.1
            ));
        }
    }
    out
}

/// The running schema of each part that extends version 12, by file name.
pub fn parts(blocks: &[Block], base: &[Entry]) -> BTreeMap<&'static str, Vec<Entry>> {
    let mut out = BTreeMap::new();
    for &(part, exts) in EXTENSIONS {
        let mut schema = base.to_vec();
        for &x in exts {
            declare(&mut schema, find(blocks, x));
        }
        out.insert(part, schema);
    }
    out
}
