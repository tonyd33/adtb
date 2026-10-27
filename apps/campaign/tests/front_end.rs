//! The schema, the migrations and the bundle, checked by adtb's front end.

use adtb_lang::ast::{BundleItem, Decl};
use adtb_lang::check::{check_query, check_schema, check_txn};
use adtb_lang::evolve::{Refusal, Source};
use adtb_lang::lex::{Tok, lex};
use adtb_lang::parse::{Options, parse_bundle, parse_migration, parse_schema};
use adtb_lang::schema::build;
use adtb_lang::span::{Diag, FileDiag, FileId};
use campaign::{BUNDLE, MIGRATIONS, MODULE, SCHEMA};

fn rendered(src: &str, ds: &[Diag]) -> String {
    ds.iter()
        .map(|d| d.render(src))
        .collect::<Vec<_>>()
        .join("\n")
}

fn in_files(src: &Source, ds: &[FileDiag]) -> String {
    ds.iter()
        .map(|d| {
            let text = if (d.file.0 as usize) < src.decls.len() {
                src.file_text(d.file)
            } else {
                String::new()
            };
            let first = text.lines().nth(1).unwrap_or_default();
            format!("in `{first}`: {}", d.diag.message)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn refused(text: &str, before: &Source, r: &Refusal) -> String {
    let result = r.result.as_deref().unwrap_or(before);
    format!(
        "{}\n{}",
        rendered(text, &r.operators),
        in_files(result, &r.recheck)
    )
}

fn migrated(before: &Source, text: &str) -> Source {
    let file = parse_migration(text, Options::default())
        .ok()
        .unwrap_or_else(|ds| panic!("the migration does not parse:\n{}", rendered(text, &ds)));
    before
        .migrate(&file, text)
        .unwrap_or_else(|r| panic!("the migration is refused:\n{}", refused(text, before, &r)))
}

/// Returns the source after the first `n` migrations.
fn version(n: usize) -> Source {
    MIGRATIONS[..n]
        .iter()
        .fold(Source::default(), |src, m| migrated(&src, m))
}

fn diags(ds: Vec<FileDiag>) -> Vec<Diag> {
    ds.into_iter().map(|d| d.diag).collect()
}

fn tokens(src: &str) -> Vec<&str> {
    lex(src)
        .0
        .into_iter()
        .filter(|t| t.tok != Tok::Eof)
        .map(|t| &src[t.span.range()])
        .collect()
}

fn named(d: &Decl) -> String {
    match d {
        Decl::Type(t) => format!("type {}", t.name.text),
        Decl::Relation(r) => format!("relation {}", r.name.name.text),
        Decl::Set(s) => format!("set {}.{}", s.relation.text, s.name.text),
        Decl::Fn(f) => format!("fn {}", f.name.text),
        Decl::Constraint(c) => format!("constraint {}", c.name.text),
        Decl::Check(c) => format!("check {}", c.name.text),
        Decl::Store(s) => format!("store {}", s.view.name.text),
        Decl::Aggregate(a) => format!("aggregate {}", a.name.text),
    }
}

fn span(d: &Decl) -> std::ops::Range<usize> {
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
    .range()
}

/// Returns each declaration of a schema file by name, as its tokens.
fn declarations(text: &str) -> Vec<(String, Vec<&str>)> {
    let file = parse_schema(text, Options::default())
        .ok()
        .unwrap_or_else(|ds| panic!("{}", rendered(text, &ds)));
    file.decls
        .iter()
        .map(|d| (named(d), tokens(&text[span(d)])))
        .collect()
}

#[test]
fn the_schema_checks() {
    let file = parse_schema(SCHEMA, Options::default())
        .ok()
        .unwrap_or_else(|ds| panic!("{}", rendered(SCHEMA, &ds)));
    let schema =
        build(&[(FileId(0), file)]).unwrap_or_else(|ds| panic!("{}", rendered(SCHEMA, &diags(ds))));
    if let Err(ds) = check_schema(&schema) {
        panic!("{}", rendered(SCHEMA, &diags(ds)));
    }
}

/// Asserts that `src` declares what the schema file declares, token for token.
fn folds_to_the_schema(src: &Source) {
    if let Err(ds) = src.check() {
        panic!("{}", in_files(src, &ds));
    }
    let texts: Vec<String> = src
        .decls
        .iter()
        .map(|d| format!("module {}\n{}", d.module, d.text))
        .collect();
    let mut folded: Vec<(String, Vec<&str>)> = texts.iter().flat_map(|t| declarations(t)).collect();
    let mut written = declarations(SCHEMA);
    folded.sort();
    written.sort();
    let differ: Vec<String> = written
        .iter()
        .filter(|w| !folded.contains(w))
        .map(|(n, _)| n.clone())
        .chain(
            folded
                .iter()
                .filter(|f| !written.iter().any(|w| w.0 == f.0))
                .map(|(n, _)| format!("{n} (only folded)")),
        )
        .collect();
    assert!(differ.is_empty(), "differ: {differ:?}");
}

#[test]
fn the_first_migration_folds_to_the_schema() {
    let v1 = version(1);
    assert_eq!(1, v1.version);
    folds_to_the_schema(&v1);
}

#[test]
fn the_schema_file_replays_as_a_first_migration() {
    let file = parse_schema(SCHEMA, Options::default())
        .ok()
        .unwrap_or_else(|ds| panic!("{}", rendered(SCHEMA, &ds)));
    let mut text = format!("module {MODULE}\nadd module {MODULE}\n");
    for d in &file.decls {
        let verb = match d {
            Decl::Fn(_) | Decl::Aggregate(_) => "define ",
            Decl::Store(_) => "",
            _ => "add ",
        };
        text.push_str(&format!("{verb}{}\n", &SCHEMA[span(d)]));
    }
    folds_to_the_schema(&migrated(&Source::default(), &text));
}

#[test]
fn release_1_1_adds_maps_and_items() {
    assert_eq!(2, version(2).version);
}

#[test]
fn release_1_2_splits_locations() {
    assert_eq!(3, version(3).version);
}

#[test]
fn release_1_3_adds_quest_states() {
    assert_eq!(4, version(4).version);
}

#[test]
fn the_bundle_parses() {
    if let Err(ds) = parse_bundle(BUNDLE, Options::default()).ok() {
        panic!("{}", rendered(BUNDLE, &ds));
    }
}

#[test]
fn every_query_and_transaction_checks_at_its_pin() {
    let bundle = parse_bundle(BUNDLE, Options::default())
        .ok()
        .unwrap_or_else(|ds| panic!("{}", rendered(BUNDLE, &ds)));
    let v1 = version(1);
    let (schema, program) = v1
        .check()
        .unwrap_or_else(|ds| panic!("{}", in_files(&v1, &ds)));
    let module = schema.module_named(MODULE).expect("the module");
    let mut failures = Vec::new();
    let mut checked = 0;
    for item in &bundle.items {
        let (name, got) = match item {
            BundleItem::Query(q) => (
                &q.name.text,
                check_query(&schema, &program, module, q).map(drop),
            ),
            BundleItem::Txn(t) => (
                &t.name.text,
                check_txn(&schema, &program, module, t).map(drop),
            ),
            _ => continue,
        };
        checked += 1;
        if let Err(d) = got {
            failures.push(format!("{name}: {}", d.render(BUNDLE)));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {checked} fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
