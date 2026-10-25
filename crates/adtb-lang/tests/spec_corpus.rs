mod running;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use adtb_lang::ast::{
    AdhocItem, BundleItem, Decl, ExampleItem, Import, MigrationFile, Operator, OperatorKind,
    QueryDecl, TxnDecl,
};
use adtb_lang::check::{
    Checker, Context, Program, check_adhoc, check_query, check_schema, check_txn,
};
use adtb_lang::evolve::{Source, SourceDecl};
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

#[test]
fn engine_source_matches_the_spec() {
    let spec: Vec<(String, String)> = blocks()
        .iter()
        .filter(|b| b.is_adt() && matches!(b.file.as_str(), "catalog.md" | "security.md"))
        .flat_map(|b| {
            let items = parse_example(&b.text, Options { elisions: true })
                .ok()
                .unwrap_or_default();
            items
                .into_iter()
                .filter_map(|i| match i {
                    ExampleItem::Decl(d) => Some((
                        format!("{} {}", decl_key(&d).0, decl_key(&d).1),
                        unqualified(&b.text[decl_span(&d).range()]),
                    )),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let mut failures = Vec::new();
    for e in prelude_entries().iter().filter(|e| e.file == "catalog.adt") {
        let name = format!("{} {}", e.key.0, e.key.1);
        let found: Vec<&String> = spec
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, t)| t)
            .collect();
        if found.is_empty() {
            failures.push(format!(
                "{}::{name} is declared nowhere in the spec",
                e.module
            ));
        } else if !found.iter().any(|t| e.spelled(t)) {
            failures.push(format!("{}::{name} differs from the spec", e.module));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn engine_source_declares_every_fixed_id() {
    let catalog = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/spec/catalog.md"),
    )
    .unwrap();
    let mut module = None;
    let mut wanted: Vec<(String, &str, String)> = Vec::new();
    for line in catalog.lines() {
        if let Some(rest) = line.strip_prefix("Module `") {
            let (m, kind) = rest.split_once("`, ").expect("Module `m`, kind:");
            module = Some((
                m.to_string(),
                if kind.starts_with("types") {
                    "type"
                } else {
                    "relation"
                },
            ));
            continue;
        }
        if line.starts_with('#') {
            module = None;
        }
        let Some((m, kind)) = &module else { continue };
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() < 3 || !cells[1].chars().all(|c| c.is_ascii_digit()) || cells[1].is_empty() {
            continue;
        }
        let name = cells[2].trim_matches('`');
        let name = name.rsplit("::").next().unwrap().to_string();
        wanted.push((m.clone(), kind, name));
    }
    assert!(!wanted.is_empty(), "no fixed ids found");
    let entries = prelude_entries();
    let missing: Vec<String> = wanted
        .iter()
        .filter(|(m, kind, name)| {
            !entries
                .iter()
                .any(|e| e.module == *m && e.key.0 == *kind && e.key.1 == *name)
        })
        .map(|(m, kind, name)| format!("{kind} {m}::{name}"))
        .collect();
    assert!(missing.is_empty(), "not declared:\n{}", missing.join("\n"));
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

/// The declarations of the prelude and of the engine's `catalog` and `sys` modules.
fn prelude_entries() -> Vec<Entry> {
    let mut out = Vec::new();
    for (src, file) in [
        (include_str!("../src/prelude.adt"), "prelude.adt"),
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
fn unqualified(text: &str) -> String {
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

fn schema_of(entries: &[Entry]) -> Result<(Schema, Program), Vec<String>> {
    let fs = files(entries)?;
    let render =
        |ds: Vec<FileDiag>| -> Vec<String> { ds.iter().map(|d| fs.render(entries, d)).collect() };
    let schema = build(&fs.files).map_err(render)?;
    let program = check_schema(&schema).map_err(render)?;
    Ok((schema, program))
}

fn at(b: &Block, span: Span, msg: &str) -> String {
    format!("{}:{}: {msg}", b.file, spec_line(b, span.start as usize))
}

/// Returns whether a comment line of the block names a migration by its number, as `-- 0013` does, so its
/// grants and revokes are operators of that migration.
fn names_a_migration(b: &Block) -> bool {
    b.text.lines().any(|l| {
        l.trim_start()
            .strip_prefix("-- ")
            .is_some_and(|rest| rest.len() >= 4 && rest[..4].chars().all(|c| c.is_ascii_digit()))
    })
}

/// Returns whether an operator only adds a declaration `entries` already holds as written, as when a block
/// shows a migration of the history its part folds in.
fn already_applied(entries: &[Entry], module: &str, b: &Block, op: &OperatorKind) -> bool {
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
fn context(b: &Block) -> Vec<Entry> {
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
fn source_of(entries: &[Entry]) -> Result<Source, Vec<String>> {
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
fn render_in(src: &Source, entries: &[Entry], b: Option<&Block>, d: &FileDiag) -> String {
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
                BundleItem::Feed(_) => {}
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
                        .map(|d| render_in(result, &entries, Some(b), d)),
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
    for q in &queries {
        if let Err(d) = check_query(&schema, &program, module, q) {
            errs.push(at(b, d.span, &d.message));
        }
    }
    for t in &txns {
        if let Err(d) = check_txn(&schema, &program, module, t) {
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
        && let Err(ds) = check_adhoc(&schema, &program, &adhoc, &params)
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
fn running_schema_is_its_history() {
    let blocks = blocks();
    let (readme, evolution) = running::version_10(&blocks);
    let mut failures = running::differences(&readme, &evolution);
    failures.extend(running::differences(
        &running::folded_12(&blocks),
        &running::version_12(&blocks),
    ));
    assert!(
        failures.is_empty(),
        "the history disagrees with the running schema:\n{}",
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
