//! The schema, the migrations and the bundles, checked by adtb's front end.

use adtb_lang::ast::{BundleItem, Decl};
use adtb_lang::check::{check_query, check_schema, check_txn};
use adtb_lang::evolve::{Refusal, Source};
use adtb_lang::lex::{Tok, lex};
use adtb_lang::parse::{Options, parse_bundle, parse_migration, parse_schema};
use adtb_lang::schema::{RelationDef, Schema, Stage, ValueDef, build};
use adtb_lang::span::{Diag, FileDiag, FileId};
use adtb_lang::types::RelId;
use campaign::{BUNDLE, BUNDLE_1_0, MIGRATIONS, MODULE, SCHEMA, WEB_BUNDLE};

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

/// Returns the checked schema after the first `n` migrations.
fn folded(n: usize) -> Schema {
    let src = version(n);
    assert_eq!(n, usize::try_from(src.version).expect("a small version"));
    src.check()
        .unwrap_or_else(|ds| panic!("{}", in_files(&src, &ds)))
        .0
}

/// Returns relation variable `name` of the campaign module.
fn relation<'a>(schema: &'a Schema, name: &str) -> (RelId, &'a RelationDef) {
    let module = schema.module_named(MODULE).expect("the module");
    match schema.module(module).values.get(name) {
        Some(&ValueDef::Relation(r)) => (r, schema.relation(r)),
        other => panic!("`{name}` is not a relation: {other:?}"),
    }
}

/// Returns the constructors of relation variable `name` in order, each with its write stage.
fn ctors<'a>(schema: &'a Schema, name: &str) -> Vec<(&'a str, Stage)> {
    let (id, rel) = relation(schema, name);
    schema
        .ty(rel.elem_def)
        .ctors()
        .iter()
        .zip(0..)
        .map(|(c, i)| (c.name.as_str(), schema.stage(id, i)))
        .collect()
}

/// Returns the named sets of relation variable `name`, each with its constructors in declared order.
fn sets<'a>(schema: &'a Schema, name: &str) -> Vec<(&'a str, Vec<&'a str>)> {
    let (_, rel) = relation(schema, name);
    let def = schema.ty(rel.elem_def);
    rel.sets
        .iter()
        .map(|s| {
            let names = s
                .ctors
                .iter()
                .map(|&c| def.ctors()[c as usize].name.as_str())
                .collect();
            (s.name.as_str(), names)
        })
        .collect()
}

/// Returns the shown type of field `field` of constructor `ctor` of relation variable `name`.
fn field_type(schema: &Schema, name: &str, ctor: &str, field: &str) -> String {
    let def = schema.ty(relation(schema, name).1.elem_def);
    let c = def
        .ctor_index(ctor)
        .unwrap_or_else(|| panic!("no constructor {name}.{ctor}"));
    let f = def
        .field(c, field)
        .unwrap_or_else(|| panic!("no field {name}.{ctor}.{field}"));
    schema.show_ty(&f.ty)
}

const ENABLED: Stage = Stage::Enabled;

#[test]
fn release_1_1_adds_maps_and_items() {
    let s = folded(2);
    assert_eq!(vec![("Map", ENABLED)], ctors(&s, "maps"));
    assert_eq!(
        vec![("OfEntity", ENABLED), ("Marker", ENABLED)],
        ctors(&s, "pins")
    );
    assert_eq!(
        vec![
            ("KnowsEntity", ENABLED),
            ("KnowsQuest", ENABLED),
            ("KnowsEvent", ENABLED),
            ("KnowsMap", ENABLED)
        ],
        ctors(&s, "discoveries")
    );
    assert_eq!(
        vec![
            ("Npc", ENABLED),
            ("Fallen", ENABLED),
            ("PlayerCharacter", ENABLED),
            ("Location", ENABLED),
            ("Faction", ENABLED),
            ("Item", ENABLED),
            ("Party", ENABLED)
        ],
        ctors(&s, "entities")
    );
    assert_eq!(
        vec![
            ("Characters", vec!["Npc", "Fallen", "PlayerCharacter"]),
            ("Groups", vec!["Faction", "Party"])
        ],
        sets(&s, "entities")
    );
    assert_eq!(
        "Option (Key entities.(Npc | Fallen | PlayerCharacter))",
        field_type(&s, "entities", "Item", "holder")
    );
}

#[test]
fn release_1_2_splits_locations() {
    let s = folded(3);
    assert_eq!(
        vec![
            ("Npc", ENABLED),
            ("Fallen", ENABLED),
            ("PlayerCharacter", ENABLED),
            ("Region", ENABLED),
            ("Settlement", ENABLED),
            ("Site", ENABLED),
            ("Faction", ENABLED),
            ("Item", ENABLED),
            ("Party", ENABLED)
        ],
        ctors(&s, "entities")
    );
    assert_eq!(
        vec![
            ("Characters", vec!["Npc", "Fallen", "PlayerCharacter"]),
            ("Groups", vec!["Faction", "Party"]),
            ("Places", vec!["Region", "Settlement", "Site"])
        ],
        sets(&s, "entities")
    );
    let places = "Option (Key entities.(Region | Settlement | Site))";
    for (ctor, field) in [
        ("Npc", "location"),
        ("Fallen", "location"),
        ("Region", "parent"),
        ("Settlement", "parent"),
        ("Site", "parent"),
    ] {
        assert_eq!(
            places,
            field_type(&s, "entities", ctor, field),
            "entities.{ctor}.{field}"
        );
    }
}

#[test]
fn release_1_3_adds_quest_states() {
    let s = folded(4);
    assert_eq!(
        vec![
            ("Rumoured", ENABLED),
            ("Active", ENABLED),
            ("Shelved", ENABLED),
            ("Completed", ENABLED),
            ("Failed", ENABLED),
            ("Abandoned", ENABLED)
        ],
        ctors(&s, "quests")
    );
    assert_eq!(
        vec![
            ("Open", vec!["Rumoured", "Active", "Shelved"]),
            ("Closed", vec!["Completed", "Failed", "Abandoned"])
        ],
        sets(&s, "quests")
    );
}

/// The bundle of each release, with the version it pins.
const BUNDLES: [(&str, usize); 3] = [(BUNDLE_1_0, 1), (BUNDLE, 4), (WEB_BUNDLE, 5)];

#[test]
fn every_bundle_parses() {
    for (text, _) in BUNDLES {
        if let Err(ds) = parse_bundle(text, Options::default()).ok() {
            panic!("{}", rendered(text, &ds));
        }
    }
}

#[test]
fn every_query_and_transaction_checks_at_its_pin() {
    for (text, pin) in BUNDLES {
        checks_at(text, pin);
    }
}

/// Assert that every query and transaction of bundle `text` checks at version `pin`.
fn checks_at(text: &str, pin: usize) {
    let bundle = parse_bundle(text, Options::default())
        .ok()
        .unwrap_or_else(|ds| panic!("{}", rendered(text, &ds)));
    assert_eq!(
        pin,
        usize::try_from(bundle.header.pins.number).expect("a small version")
    );
    let at = version(pin);
    let (schema, program) = at
        .check()
        .unwrap_or_else(|ds| panic!("{}", in_files(&at, &ds)));
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
            failures.push(format!("{name}: {}", d.render(text)));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {checked} at version {pin} fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn browser_bundle_pins_the_current_schema_hash() {
    let source = version(MIGRATIONS.len());
    let schema = source.schema().unwrap();
    let hash = adtb::catalog::version_hash(&schema, &source);
    let hex: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
    let bundle = parse_bundle(WEB_BUNDLE, Options::default()).ok().unwrap();
    assert_eq!(bundle.header.pins.hash.as_deref(), Some(hex.as_str()));
}
