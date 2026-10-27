//! The examples of the spec, each with the schema it is checked against.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use adtb_lang::ast::{
    AdhocItem, BundleItem, Decl, ExampleItem, FeedDecl, Import, MigrationFile, Operator,
    OperatorKind, QueryDecl, TxnDecl,
};
use adtb_lang::check::{Bundles, Checker, Context, Program, check_feed};
use adtb_lang::evolve::{Source, SourceDecl};
use adtb_lang::parse::{Options, parse_example, parse_type};
use adtb_lang::resolve::Visible;
use adtb_lang::schema::Schema;
use adtb_lang::span::{FileDiag, Span};
use adtb_lang::types::{ModuleId, Ty};

use crate::running::{self, Entry, decl_key, decl_span, spec_line};

pub struct Block {
    pub file: String,
    pub line: usize,
    pub info: String,
    pub text: String,
}

pub fn blocks() -> Vec<Block> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/spec");
    let mut paths: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let src = fs::read_to_string(&path).unwrap();
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        let mut current: Option<Block> = None;
        for (i, line) in src.lines().enumerate() {
            let trimmed = line.trim_start();
            if let Some(info) = trimmed.strip_prefix("```") {
                match current.take() {
                    Some(b) => out.push(b),
                    None => {
                        current = Some(Block {
                            file: file.clone(),
                            line: i + 1,
                            info: info.trim().to_string(),
                            text: String::new(),
                        });
                    }
                }
            } else if let Some(b) = current.as_mut() {
                b.text.push_str(line);
                b.text.push('\n');
            }
        }
    }
    out
}

impl Block {
    pub fn is_adt(&self) -> bool {
        self.info == "adt" || self.info == FRAGMENT
    }
}

pub const FRAGMENT: &str = "adt fragment";

/// The ad hoc parameters a block's `-- given` header declares, as name and type text.
pub fn given(b: &Block) -> Vec<(String, String)> {
    let Some(header) = b
        .text
        .lines()
        .take_while(|l| l.trim_start().starts_with("--"))
        .find_map(|l| l.trim_start().strip_prefix("-- given "))
    else {
        return Vec::new();
    };
    let mut params = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in header.char_indices() {
        match c {
            '(' | '{' => depth += 1,
            ')' | '}' => depth -= 1,
            ',' if depth == 0 => {
                params.push(&header[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    params.push(&header[start..]);
    params
        .into_iter()
        .map(|p| {
            let (name, ty) = p.split_once(':').expect("`name: Type`");
            (name.trim().to_string(), ty.trim().to_string())
        })
        .collect()
}

/// The declarations of the prelude and of the engine's `catalog` and `sys` modules.
pub fn prelude_entries() -> Vec<Entry> {
    let mut out = Vec::new();
    for (src, file) in [
        (include_str!("../../src/prelude.adt"), "prelude.adt"),
        (adtb_lang::schema::ENGINE_SRC, "catalog.adt"),
    ] {
        let mut module = String::new();
        for item in parse_example(src, Options::default()).ok().expect("parses") {
            match item {
                ExampleItem::Module(m) => module = m.text,
                ExampleItem::Decl(d) => out.push(Entry {
                    module: module.clone(),
                    key: decl_key(&d),
                    text: unqualified(&src[decl_span(&d).range()]),
                    file: file.to_string(),
                    line: 0,
                }),
                _ => panic!("{file} holds only modules and declarations"),
            }
        }
    }
    out
}

/// The text of a declaration with the module qualifier of a relation name removed.
pub fn unqualified(text: &str) -> String {
    let toks = running::tokens(text);
    match toks.as_slice() {
        ["relation", m, "::", ..] => {
            let at = text.find(m).expect("the qualifier");
            let rest = &text[at + m.len()..];
            format!(
                "relation {}",
                rest.trim_start().trim_start_matches("::").trim_start()
            )
        }
        _ => text.to_string(),
    }
}

pub fn at(b: &Block, span: Span, msg: &str) -> String {
    format!("{}:{}: {msg}", b.file, spec_line(b, span.start as usize))
}

/// Returns whether a comment line of the block names a migration by its number, as `-- 0013` does, so its
/// grants and revokes are operators of that migration.
pub fn names_a_migration(b: &Block) -> bool {
    b.text.lines().any(|l| {
        l.trim_start()
            .strip_prefix("-- ")
            .is_some_and(|rest| rest.len() >= 4 && rest[..4].chars().all(|c| c.is_ascii_digit()))
    })
}

/// Returns whether an operator only adds a declaration `entries` already holds as written, as when a block
/// shows a migration of the history its part folds in.
pub fn already_applied(entries: &[Entry], module: &str, b: &Block, op: &OperatorKind) -> bool {
    let d = match op {
        OperatorKind::AddType(t) => Decl::Type(t.clone()),
        OperatorKind::AddRelation(r) => Decl::Relation(r.clone()),
        OperatorKind::AddSet(s) => Decl::Set(s.clone()),
        OperatorKind::DefineFn(f) => Decl::Fn(f.clone()),
        OperatorKind::AddConstraint(c) => Decl::Constraint(c.clone()),
        OperatorKind::AddCheck(c) => Decl::Check(c.clone()),
        OperatorKind::Store(s) => Decl::Store(s.clone()),
        OperatorKind::DefineAggregate(a) => Decl::Aggregate(a.clone()),
        _ => return false,
    };
    let key = decl_key(&d);
    let text = &b.text[decl_span(&d).range()];
    entries
        .iter()
        .any(|e| e.module == module && e.key == key && e.spelled(text))
}

/// The declarations a block's comment states as its context, indented under a comment line ending in
/// `with` or `with:`.
pub fn context(b: &Block) -> Vec<Entry> {
    let header: Vec<&str> = b.text.lines().collect();
    let Some(start) = header.iter().position(|l| {
        let l = l.trim_end();
        l.starts_with("--") && (l.ends_with(" with") || l.ends_with("with:"))
    }) else {
        return Vec::new();
    };
    let text: String = header[start + 1..]
        .iter()
        .map_while(|l| l.strip_prefix("--   "))
        .map(|l| format!("{l}\n"))
        .collect();
    let line = b.line + 2 + start;
    let parsed = parse_example(&text, Options { elisions: true })
        .ok()
        .unwrap_or_else(|_| panic!("{}:{line}: the context does not parse", b.file));
    parsed
        .into_iter()
        .filter_map(|i| match i {
            ExampleItem::Decl(d) => Some(Entry {
                module: "auth".to_string(),
                key: decl_key(&d),
                text: text[decl_span(&d).range()].to_string(),
                file: b.file.clone(),
                line: line + text[..decl_span(&d).start as usize].matches('\n').count(),
            }),
            _ => None,
        })
        .collect()
}

/// The source of the declarations `entries`, declaration `i` holding entry `i`.
pub fn source_of(entries: &[Entry]) -> Result<Source, Vec<String>> {
    let decls = entries
        .iter()
        .map(|e| SourceDecl {
            module: e.module.clone(),
            text: e.text.clone(),
        })
        .collect();
    Source::new(decls).map_err(|ds| {
        let src = Source {
            decls: entries
                .iter()
                .map(|e| SourceDecl {
                    module: e.module.clone(),
                    text: e.text.clone(),
                })
                .collect(),
            ..Source::default()
        };
        ds.iter()
            .map(|d| render_in(&src, entries, None, d))
            .collect()
    })
}

/// Renders a diagnostic in a file of `src`: at the spec line of its entry when the declaration is the entry's
/// text unchanged, else at block `b` naming the declaration.
pub fn render_in(src: &Source, entries: &[Entry], b: Option<&Block>, d: &FileDiag) -> String {
    if d.file.0 as usize >= src.decls.len() {
        return d.diag.message.clone();
    }
    let i = d.file.0 as usize;
    let text = src.file_text(d.file);
    let at = (d.diag.span.start as usize).min(text.len());
    let line_in = text[..at].matches('\n').count();
    match entries.get(i) {
        Some(e) if e.text == src.decls[i].text && e.module == src.decls[i].module => {
            format!(
                "{}:{}: {}",
                e.file,
                e.line + line_in.saturating_sub(1),
                d.diag.message
            )
        }
        _ => {
            let first = src.decls[i].text.lines().next().unwrap_or_default();
            match b {
                Some(b) => format!("{}:{}: in `{first}`: {}", b.file, b.line, d.diag.message),
                None => format!("in `{first}`: {}", d.diag.message),
            }
        }
    }
}

/// A registered query some example declares: its bundle's family and version, and its declaration.
pub type Registered = (String, String, QueryDecl);

/// Returns every registered query the examples declare under a bundle header.
pub fn registered_queries(blocks: &[Block]) -> Vec<Registered> {
    let mut out = Vec::new();
    for b in blocks.iter().filter(|b| b.is_adt() && b.info != FRAGMENT) {
        let Ok(items) = parse_example(&b.text, Options { elisions: true }).ok() else {
            continue;
        };
        let mut bundle = None;
        for item in items {
            match item {
                ExampleItem::BundleHeader(h) => bundle = Some(h.name),
                ExampleItem::BundleItem(BundleItem::Query(q)) => {
                    if let Some(n) = &bundle {
                        out.push((n.family.clone(), n.version.clone(), q));
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// A block made ready to check: the schema its context and declarations build, the module its
/// bundle reads, and the queries, transactions and ad hoc items it holds.
pub struct Prepared {
    pub schema: Schema,
    pub program: Program,
    pub module: ModuleId,
    pub queries: Vec<QueryDecl>,
    pub txns: Vec<TxnDecl>,
    pub adhoc: Vec<AdhocItem>,
}

/// Builds the schema of block `b` over the running schema `part`, applying the migrations it shows.
pub fn prepare(b: &Block, part: &[Entry], prelude: &[Entry]) -> Result<Prepared, Vec<String>> {
    let items = parse_example(&b.text, Options { elisions: true })
        .ok()
        .map_err(|ds| {
            vec![format!(
                "{}:{}: does not parse: {}",
                b.file, b.line, ds[0].message
            )]
        })?;
    let in_migration = names_a_migration(b);
    let mut entries = part.to_vec();
    for d in context(b) {
        entries.retain(|e| !(e.module == d.module && e.key == d.key));
        entries.push(d);
    }
    let mut module = "auth".to_string();
    let mut reads = None;
    let mut adhoc = Vec::new();
    let mut queries: Vec<QueryDecl> = Vec::new();
    let mut txns: Vec<TxnDecl> = Vec::new();
    let mut feeds: Vec<FeedDecl> = Vec::new();
    let mut migrations: Vec<MigrationFile> = Vec::new();
    let mut errs = Vec::new();
    let declare = |entries: &mut Vec<Entry>, module: &str, d: Decl| {
        let key = decl_key(&d);
        let text = unqualified(&b.text[decl_span(&d).range()]);
        if !prelude.iter().any(|e| e.key == key && e.spelled(&text)) {
            running::push(entries, module, b, &d);
        }
    };
    let ident = |m: &str| adtb_lang::ast::Ident {
        text: m.to_string(),
        span: Span::default(),
    };
    for item in items {
        match item {
            ExampleItem::Module(m) => module = m.text,
            ExampleItem::Decl(Decl::Store(st)) if !migrations.is_empty() => {
                let m = migrations.last_mut().expect("a migration");
                m.operators.push(Operator {
                    span: st.span,
                    kind: OperatorKind::Store(st),
                });
            }
            ExampleItem::Decl(d) => declare(&mut entries, &module, d),
            ExampleItem::Operator(op) => match op.kind {
                OperatorKind::Grant(g) if !in_migration => adhoc.push(AdhocItem::Grant(g)),
                OperatorKind::Revoke(r) if !in_migration => adhoc.push(AdhocItem::Revoke(r)),
                _ if already_applied(&entries, &module, b, &op.kind) => {}
                _ => match migrations.last_mut() {
                    Some(m) if m.module.text == module => m.operators.push(op),
                    _ => migrations.push(MigrationFile {
                        module: ident(&module),
                        operators: vec![op],
                    }),
                },
            },
            ExampleItem::BundleHeader(_) | ExampleItem::Import(Import::Fn(_)) => {}
            ExampleItem::Import(Import::Module(m)) => reads = Some(m.text),
            ExampleItem::BundleItem(i) => match i {
                BundleItem::Fn(f) => declare(&mut entries, &module, Decl::Fn(f)),
                BundleItem::Aggregate(a) => declare(&mut entries, &module, Decl::Aggregate(a)),
                BundleItem::Query(q) => queries.push(q),
                BundleItem::Txn(t) => txns.push(t),
                BundleItem::Feed(f) => feeds.push(f),
            },
            ExampleItem::Adhoc(a) => adhoc.push(a),
        }
    }
    let mut src = match source_of(&entries) {
        Ok(s) => s,
        Err(mut es) => {
            errs.append(&mut es);
            return Err(errs);
        }
    };
    for m in &migrations {
        match src.migrate(m, &b.text) {
            Ok(next) => src = next,
            Err(refusal) => {
                errs.extend(refusal.operators.iter().map(|d| at(b, d.span, &d.message)));
                let result = refusal.result.as_deref().unwrap_or(&src);
                errs.extend(
                    refusal
                        .recheck
                        .iter()
                        .zip(&refusal.recheck_from)
                        .map(|(d, from)| match from {
                            Some(span) => at(b, *span, &d.diag.message),
                            None => render_in(result, &entries, Some(b), d),
                        }),
                );
                return Err(errs);
            }
        }
    }
    let (schema, program) = match src.check() {
        Ok(s) => s,
        Err(ds) => {
            errs.extend(ds.iter().map(|d| render_in(&src, &entries, Some(b), d)));
            return Err(errs);
        }
    };
    let reads = reads.unwrap_or_else(|| "auth".to_string());
    let module = schema
        .module_named(&reads)
        .expect("the bundle reads a module");
    for f in &feeds {
        if let Err(d) = check_feed(&schema, module, f) {
            errs.push(at(b, d.span, &d.message));
        }
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    Ok(Prepared {
        schema,
        program,
        module,
        queries,
        txns,
        adhoc,
    })
}

/// Returns the types of the ad hoc parameters block `b` declares, resolved in `schema`, and an error
/// for each it cannot resolve.
pub fn given_types(b: &Block, schema: &Schema) -> (BTreeMap<String, Ty>, Vec<String>) {
    let mut params = BTreeMap::new();
    let mut errs = Vec::new();
    let c = Checker::new(schema, Visible::all(schema), Context::Adhoc);
    for (name, ty) in given(b) {
        let Ok(parsed) = parse_type(&ty, Options::default()).ok() else {
            errs.push(format!("{}:{}: `{ty}` is not a type", b.file, b.line));
            continue;
        };
        match c.resolve_type(&parsed) {
            Ok(t) => {
                params.insert(name, t);
            }
            Err(d) => errs.push(format!(
                "{}:{}: given `{name}`: {}",
                b.file, b.line, d.message
            )),
        }
    }
    (params, errs)
}

/// Returns the bundles of the `registered` queries whose parameter types resolve in `schema`.
pub fn bundles(schema: &Schema, registered: &[Registered]) -> Bundles {
    let c = Checker::new(schema, Visible::all(schema), Context::Adhoc);
    let mut bundles = Bundles::default();
    for (family, version, q) in registered {
        let params: Result<Vec<_>, _> = q.params.iter().map(|p| c.resolve_type(&p.ty)).collect();
        if let Ok(params) = params {
            bundles.insert(family, version, &q.name.text, params);
        }
    }
    bundles
}
