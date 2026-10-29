use std::fs;
use std::path::Path;

use adtb_lang::ast::ExampleItem;
use adtb_lang::check::{Program, check_adhoc_recording, check_query, check_schema, check_txn};
use adtb_lang::parse::{Options, parse_example, parse_schema};
use adtb_lang::schema::{Schema, build};
use adtb_lang::span::{FileDiag, FileId};
use adtb_testkit::running::{self, Entry, decl_key, decl_span};
use adtb_testkit::spec::{
    Block, FRAGMENT, Prepared, Registered, at, blocks, bundles, given_types, prelude_entries,
    prepare, registered_queries, unqualified,
};

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
                Some((t.name.text, words.join(" ")))
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

#[test]
fn engine_ids_are_the_fixed_ids_of_the_spec() {
    let catalog = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/spec/catalog.md"),
    )
    .unwrap();
    let schema = build(&[]).expect("the engine's declarations build");
    let mut module: Option<(String, bool)> = None;
    let mut failures = Vec::new();
    let mut seen = 0;
    for line in catalog.lines() {
        if line.starts_with("Prelude:") {
            module = Some(("prelude".to_string(), true));
            continue;
        }
        if let Some(rest) = line.strip_prefix("Module `") {
            let (m, kind) = rest.split_once("`, ").expect("Module `m`, kind:");
            module = Some((m.to_string(), kind.starts_with("types")));
            continue;
        }
        if line.starts_with('#') {
            module = None;
        }
        let Some((m, types)) = &module else { continue };
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        let Some(Ok(id)) = cells.get(1).map(|c| c.parse::<u64>()) else {
            continue;
        };
        seen += 1;
        let name = cells[2].split(',').next().unwrap().trim_matches('`');
        let name = name.rsplit("::").next().unwrap();
        let mid = schema.module_named(m).expect("an engine module");
        if !types {
            let got = match schema.module(mid).values.get(name) {
                Some(&adtb_lang::schema::ValueDef::Relation(r)) => schema.ids.relations.get(&r),
                _ => None,
            };
            if got != Some(&id) {
                failures.push(format!("relation {m}::{name}: {got:?}, not {id}"));
            }
            continue;
        }
        let Some(&t) = schema.module(mid).types.get(name) else {
            if name != "Tag" {
                failures.push(format!("no type {m}::{name}"));
            }
            continue;
        };
        if schema.ids.types.get(&t) != Some(&id) {
            failures.push(format!(
                "type {m}::{name}: {:?}, not {id}",
                schema.ids.types.get(&t)
            ));
        }
        let td = schema.ty(t);
        let ctors: Vec<(String, u64)> = if cells[3].contains("one constructor") {
            vec![(
                name.to_string(),
                cells[3].split_whitespace().next().unwrap().parse().unwrap(),
            )]
        } else {
            cells[3]
                .split(',')
                .filter_map(|p| {
                    let mut w = p.split_whitespace();
                    let id = w.next()?.parse().ok()?;
                    Some((w.next()?.trim_matches('`').to_string(), id))
                })
                .collect()
        };
        for (c, cid) in ctors {
            let newtype = matches!(td.kind, adtb_lang::schema::TypeDefKind::Newtype { .. });
            let index = if newtype { Some(0) } else { td.ctor_index(&c) };
            let got = index.and_then(|k| schema.ids.ctors.get(&(t, k)));
            if got != Some(&cid) {
                failures.push(format!("constructor {m}::{name}.{c}: {got:?}, not {cid}"));
            }
        }
    }
    assert!(seen > 100, "the id tables were not found");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
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

/// Type checks block `b` against the running schema `part`.
fn check_block(
    b: &Block,
    part: &[Entry],
    prelude: &[Entry],
    registered: &[Registered],
) -> Result<(), Vec<String>> {
    let Prepared {
        schema,
        program,
        module,
        queries,
        txns,
        adhoc,
    } = prepare(b, part, prelude)?;
    let mut errs = Vec::new();
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
    let (params, mut unresolved) = given_types(b, &schema);
    errs.append(&mut unresolved);
    let (bundles, mut unresolved) = bundles(&schema, registered);
    errs.append(&mut unresolved);
    if !adhoc.is_empty()
        && let Err(ds) = check_adhoc_recording(&schema, &program, &adhoc, &params, &bundles, None)
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
    let registered = registered_queries(&blocks);
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
        if let Err(es) = check_block(b, part, &prelude, &registered) {
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
