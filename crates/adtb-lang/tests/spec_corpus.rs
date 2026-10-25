mod running;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use adtb_lang::ast::{AdhocItem, BundleItem, Decl, ExampleItem, Import, OperatorKind, QueryDecl};
use adtb_lang::check::{Checker, Context, check_adhoc, check_query, check_schema};
use adtb_lang::parse::{Options, parse_example, parse_schema, parse_type};
use adtb_lang::resolve::Visible;
use adtb_lang::schema::{Schema, build};
use adtb_lang::span::{FileDiag, FileId, Span};
use running::{Entry, decl_key, decl_span, spec_line};

struct Block {
    file: String,
    line: usize,
    info: String,
    text: String,
}

fn blocks() -> Vec<Block> {
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
                        })
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
    fn is_adt(&self) -> bool {
        self.info == "adt" || self.info == FRAGMENT
    }
}

const FRAGMENT: &str = "adt fragment";

#[test]
fn spec_examples_parse() {
    let opts = Options { elisions: true };
    let mut failures = Vec::new();
    let mut checked = 0;
    for b in blocks().iter().filter(|b| b.is_adt()) {
        checked += 1;
        let parsed = parse_example(&b.text, opts);
        if !parsed.diags.is_empty() {
            let msgs: Vec<String> = parsed.diags.iter().map(|d| d.render(&b.text)).collect();
            failures.push(format!("{}:{}: {}", b.file, b.line, msgs.join("; ")));
        }
    }
    assert!(checked > 0, "no `adt` blocks found");
    assert!(
        failures.is_empty(),
        "{} of {checked} examples fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_spec_block_names_its_kind() {
    let kinds = ["adt", FRAGMENT, "ebnf", "text", "sh", "rust", "toml"];
    let bad: Vec<String> = blocks()
        .iter()
        .filter(|b| !kinds.contains(&b.info.as_str()))
        .map(|b| format!("{}:{} `{}`", b.file, b.line, b.info))
        .collect();
    assert!(
        bad.is_empty(),
        "blocks without a known kind:\n{}",
        bad.join("\n")
    );
}

fn normalized_decls(src: &str) -> Vec<(String, String)> {
    use adtb_lang::ast::{Decl, ExampleItem};
    let Ok(items) = parse_example(src, Options { elisions: true }).ok() else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|i| match i {
            ExampleItem::Decl(Decl::Type(t)) => {
                let text = &src[t.span.range()];
                let words: Vec<&str> = text
                    .lines()
                    .map(|l| l.split("--").next().unwrap())
                    .flat_map(str::split_whitespace)
                    .collect();
                Some((t.name.text.clone(), words.join(" ")))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn prelude_source_matches_the_spec() {
    let prelude = include_str!("../src/prelude.adt");
    let ours = normalized_decls(prelude);
    let spec: Vec<(String, String)> = blocks()
        .iter()
        .filter(|b| {
            b.is_adt() && matches!(b.file.as_str(), "prelude.md" | "catalog.md" | "interop.md")
        })
        .flat_map(|b| normalized_decls(&b.text))
        .collect();
    for (name, text) in &ours {
        let found: Vec<&String> = spec
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, t)| t)
            .collect();
        if name == "Principal" {
            continue;
        }
        assert!(
            !found.is_empty(),
            "prelude type `{name}` is declared nowhere in the spec"
        );
        assert!(
            found.iter().all(|t| *t == text),
            "prelude type `{name}` differs from the spec:\n ours: {text}\n spec: {found:?}"
        );
    }
}

/// The ad hoc parameters a block's `-- given` header declares, as name and type text.
fn given(b: &Block) -> Vec<(String, String)> {
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

fn prelude_entries() -> Vec<Entry> {
    let src = include_str!("../src/prelude.adt");
    let file = parse_schema(src, Options::default()).ok().expect("parses");
    file.decls
        .iter()
        .map(|d| Entry {
            module: "prelude".to_string(),
            key: decl_key(d),
            text: src[decl_span(d).range()].to_string(),
            file: "prelude.adt".to_string(),
            line: 0,
        })
        .collect()
}

/// The schema files of `entries`, one per module, each with the entries it holds by offset.
struct Files {
    files: Vec<(FileId, adtb_lang::ast::SchemaFile)>,
    texts: Vec<String>,
    offsets: Vec<Vec<(usize, usize)>>,
}

fn files(entries: &[Entry]) -> Result<Files, Vec<String>> {
    let mut modules: Vec<&str> = Vec::new();
    for e in entries {
        if !modules.contains(&e.module.as_str()) {
            modules.push(&e.module);
        }
    }
    let mut out = Files {
        files: Vec::new(),
        texts: Vec::new(),
        offsets: Vec::new(),
    };
    let mut errs = Vec::new();
    for (i, m) in modules.iter().enumerate() {
        let mut text = format!("module {m}\n");
        let mut offsets = Vec::new();
        for (j, e) in entries.iter().enumerate().filter(|(_, e)| e.module == *m) {
            offsets.push((text.len(), j));
            text.push_str(&e.text);
            text.push('\n');
        }
        let parsed = parse_schema(&text, Options { elisions: true });
        out.offsets.push(offsets);
        out.texts.push(text);
        match parsed.ok() {
            Ok(f) => out.files.push((FileId(i as u32), f)),
            Err(ds) => {
                for d in ds {
                    errs.push(out.render(
                        entries,
                        &FileDiag {
                            file: FileId(i as u32),
                            diag: d,
                        },
                    ));
                }
            }
        }
    }
    if errs.is_empty() { Ok(out) } else { Err(errs) }
}

impl Files {
    /// Renders a diagnostic in a schema file at the spec line its entry came from.
    fn render(&self, entries: &[Entry], d: &FileDiag) -> String {
        if d.file == FileId::PRELUDE {
            return format!("prelude: {}", d.diag.message);
        }
        let i = d.file.0 as usize;
        let at = d.diag.span.start as usize;
        let &(start, j) = self.offsets[i]
            .iter()
            .rev()
            .find(|(start, _)| *start <= at)
            .expect("an entry holds every span");
        let e = &entries[j];
        let line = e.line + self.texts[i][start..at].matches('\n').count();
        format!("{}:{line}: {}", e.file, d.diag.message)
    }
}

fn schema_of(entries: &[Entry]) -> Result<Schema, Vec<String>> {
    let fs = files(entries)?;
    let render =
        |ds: Vec<FileDiag>| -> Vec<String> { ds.iter().map(|d| fs.render(entries, d)).collect() };
    let schema = build(&fs.files).map_err(render)?;
    check_schema(&schema).map_err(render)?;
    Ok(schema)
}

fn at(b: &Block, span: Span, msg: &str) -> String {
    format!("{}:{}: {msg}", b.file, spec_line(b, span.start as usize))
}

/// Type checks block `b` against the running schema `part`.
fn check_block(b: &Block, part: &[Entry], prelude: &[Entry]) -> Result<(), Vec<String>> {
    let items = parse_example(&b.text, Options { elisions: true })
        .ok()
        .map_err(|ds| {
            vec![format!(
                "{}:{}: does not parse: {}",
                b.file, b.line, ds[0].message
            )]
        })?;
    let mut entries = part.to_vec();
    let mut module = "auth".to_string();
    let mut reads = None;
    let mut adhoc = Vec::new();
    let mut queries: Vec<QueryDecl> = Vec::new();
    let mut errs = Vec::new();
    let declare = |entries: &mut Vec<Entry>, module: &str, d: Decl| {
        let key = decl_key(&d);
        let text = &b.text[decl_span(&d).range()];
        if !prelude.iter().any(|e| e.key == key && e.spelled(text)) {
            running::push(entries, module, b, &d);
        }
    };
    for item in items {
        match item {
            ExampleItem::Module(m) => module = m.text,
            ExampleItem::Decl(d) => declare(&mut entries, &module, d),
            ExampleItem::Operator(op) => match op.kind {
                OperatorKind::AddModule(_) => {}
                OperatorKind::Grant(g) => adhoc.push(AdhocItem::Grant(g)),
                OperatorKind::Revoke(r) => adhoc.push(AdhocItem::Revoke(r)),
                OperatorKind::AddType(t) => declare(&mut entries, &module, Decl::Type(t)),
                OperatorKind::AddRelation(r) => declare(&mut entries, &module, Decl::Relation(r)),
                OperatorKind::AddSet(s) => declare(&mut entries, &module, Decl::Set(s)),
                OperatorKind::DefineFn(f) => declare(&mut entries, &module, Decl::Fn(f)),
                OperatorKind::RedefineFn(f) => {
                    let name = f.name.text.clone();
                    entries.retain(|e| !(e.module == module && e.key == ("fn", name.clone())));
                    declare(&mut entries, &module, Decl::Fn(f));
                }
                OperatorKind::AddConstraint(c) => {
                    declare(&mut entries, &module, Decl::Constraint(c))
                }
                OperatorKind::AddCheck(c) => declare(&mut entries, &module, Decl::Check(c)),
                OperatorKind::Store(s) => declare(&mut entries, &module, Decl::Store(s)),
                OperatorKind::DefineAggregate(a) => {
                    declare(&mut entries, &module, Decl::Aggregate(a))
                }
                _ => errs.push(at(b, op.span, "migration operators are not checked yet")),
            },
            ExampleItem::BundleHeader(_) | ExampleItem::Import(Import::Fn(_)) => {}
            ExampleItem::Import(Import::Module(m)) => reads = Some(m.text),
            ExampleItem::BundleItem(i) => match i {
                BundleItem::Fn(f) => declare(&mut entries, &module, Decl::Fn(f)),
                BundleItem::Aggregate(a) => declare(&mut entries, &module, Decl::Aggregate(a)),
                BundleItem::Query(q) => queries.push(q),
                BundleItem::Txn(t) => errs.push(at(b, t.span, "writes are not checked yet")),
                BundleItem::Feed(_) => {}
            },
            ExampleItem::Adhoc(a) => adhoc.push(a),
        }
    }
    let schema = match schema_of(&entries) {
        Ok(s) => s,
        Err(mut es) => {
            errs.append(&mut es);
            return Err(errs);
        }
    };
    let reads = reads.unwrap_or_else(|| "auth".to_string());
    let module = schema
        .module_named(&reads)
        .expect("the bundle reads a module");
    for q in &queries {
        if let Err(d) = check_query(&schema, module, q) {
            errs.push(at(b, d.span, &d.message));
        }
    }
    let mut params = BTreeMap::new();
    let c = Checker::new(&schema, Visible::all(&schema), Context::Adhoc);
    for (name, ty) in given(b) {
        let parsed = parse_type(&ty, Options::default())
            .ok()
            .map_err(|_| vec![format!("{}:{}: `{ty}` is not a type", b.file, b.line)])?;
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
    if !adhoc.is_empty()
        && let Err(ds) = check_adhoc(&schema, &adhoc, &params)
    {
        errs.extend(ds.iter().map(|d| at(b, d.span, &d.message)));
    }
    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

#[test]
fn running_schemas_check() {
    let blocks = blocks();
    let base = running::version_12(&blocks);
    let mut failures = Vec::new();
    if let Err(es) = schema_of(&base) {
        failures.push(format!("version 12: {}", es.join("; ")));
    }
    for (part, entries) in running::parts(&blocks, &base) {
        if let Err(es) = schema_of(&entries) {
            failures.push(format!("{part}: {}", es.join("; ")));
        }
    }
    assert!(
        failures.is_empty(),
        "running schemas fail:\n{}",
        failures.join("\n")
    );
}

#[test]
fn spec_examples_check() {
    let blocks = blocks();
    let base = running::version_12(&blocks);
    let parts = running::parts(&blocks, &base);
    let prelude = prelude_entries();
    let mut failures = Vec::new();
    let mut checked = 0;
    let mut fragments = 0;
    for b in blocks.iter().filter(|b| b.is_adt()) {
        if b.info == FRAGMENT {
            fragments += 1;
            continue;
        }
        checked += 1;
        let part = parts.get(b.file.as_str()).unwrap_or(&base);
        if let Err(es) = check_block(b, part, &prelude) {
            failures.push(format!("{}:{}:\n  {}", b.file, b.line, es.join("\n  ")));
        }
    }
    assert!(checked > 0, "no `adt` blocks found");
    assert!(
        failures.is_empty(),
        "{} of {checked} examples fail ({fragments} fragments):\n{}",
        failures.len(),
        failures.join("\n")
    );
}
