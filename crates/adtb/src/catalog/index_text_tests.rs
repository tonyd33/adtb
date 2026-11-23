use std::collections::BTreeSet;

use adtb_lang::ast::{CallArgs, ExprKind, Lit};
use adtb_lang::evolve::Source;
use adtb_lang::parse::{Options, parse_expr, parse_migration};
use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::types::RelId;
use adtb_lang::visit::each_expr;
use adtb_sim::{Config, Sim, SimRuntime};

use crate::catalog::{Catalog, Migrating, Snapshot, change, delete_rows, layout};
use crate::key::Value;
use crate::mvcc::{Scope, Transactions};
use crate::storage::Pager;
use crate::test_world::{finished, options};

const TYPES: &str = "module app
add module app
add type Status = Draft | Sent { at: I64 }
add type Label = { id: Key tags, item: Key items, n: I64 }
add type Item = { id: Key items, name: Text, n: I64, f: F64, d: Decimal 2, s: Status,
  when: Instant }
define fn twice (x: I64) -> I64 = x * 2
define fn apply (g: I64 -> Bool, x: I64) -> Bool = g(x)
add relation tags : Label key id generate sequence
";

/// Returns the migration declaring `items` with one index over `element`.
fn declaring(element: &str) -> String {
    format!("{TYPES}add relation items : Item key id generate sequence\n  index ({element})\n")
}

/// Every form `form` names but `elided`, which a migration does not parse.
const FORMS: [&str; 30] = [
    "aggregate",
    "at",
    "backlink",
    "binary",
    "bool",
    "call",
    "comprehension",
    "constructor",
    "decimal",
    "duration",
    "exists",
    "field",
    "fix",
    "float",
    "if",
    "int",
    "key call",
    "lambda",
    "match",
    "name",
    "record",
    "reject",
    "select",
    "set",
    "text",
    "top",
    "type call",
    "unary",
    "unit",
    "write",
];

fn form(kind: &ExprKind) -> &'static str {
    match kind {
        ExprKind::Lit(Lit::Int(_)) => "int",
        ExprKind::Lit(Lit::Decimal(_)) => "decimal",
        ExprKind::Lit(Lit::Float(_)) => "float",
        ExprKind::Lit(Lit::Text(_)) => "text",
        ExprKind::Lit(Lit::Duration { .. }) => "duration",
        ExprKind::Lit(Lit::Bool(_)) => "bool",
        ExprKind::Name(_) => "name",
        ExprKind::Call {
            args: CallArgs::List(_),
            ..
        } => "call",
        ExprKind::Call {
            args: CallArgs::Aggregate { .. },
            ..
        } => "aggregate",
        ExprKind::TypeCall { .. } => "type call",
        ExprKind::Ctor { .. } => "constructor",
        ExprKind::Record(_) => "record",
        ExprKind::Set(_) => "set",
        ExprKind::Unit => "unit",
        ExprKind::Unary(..) => "unary",
        ExprKind::Binary(..) => "binary",
        ExprKind::Field(..) => "field",
        ExprKind::Select(..) => "select",
        ExprKind::Backlink(..) => "backlink",
        ExprKind::KeyCall(..) => "key call",
        ExprKind::Comprehension(_) => "comprehension",
        ExprKind::Match(..) => "match",
        ExprKind::Fix(..) => "fix",
        ExprKind::Lambda(..) => "lambda",
        ExprKind::Exists(..) => "exists",
        ExprKind::Top { .. } => "top",
        ExprKind::If(..) => "if",
        ExprKind::At { .. } => "at",
        ExprKind::Reject(_) => "reject",
        ExprKind::Write(_) => "write",
        ExprKind::Elided => "elided",
    }
}

/// Returns the form of each expression of index element `text`.
fn forms_of(text: &str) -> BTreeSet<&'static str> {
    let e = parse_expr(text, Options::default())
        .ok()
        .unwrap_or_else(|ds| panic!("`{text}` parses: {ds:?}"));
    let mut out = BTreeSet::new();
    each_expr(&e, &mut |x| {
        out.insert(form(&x.kind));
    });
    out
}

#[test]
fn the_index_element_scripts_reach_every_expression_form() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../adtb-sim/scripts/catalog/rows");
    let (mut forms, mut elements) = (BTreeSet::new(), 0);
    for name in [
        "index-elements-canonical.test",
        "index-elements-refused.test",
    ] {
        let text = std::fs::read_to_string(dir.join(name)).expect("the script reads");
        let declared = text
            .lines()
            .filter_map(|l| l.trim_start().strip_prefix("index (")?.strip_suffix(')'));
        for element in declared {
            forms.extend(forms_of(element));
            elements += 1;
        }
    }
    assert_eq!(28, elements);
    assert_eq!(BTreeSet::from(FORMS), forms);
}

fn rel(catalog: &Catalog, module: &str, name: &str) -> RelId {
    relation(catalog.schema(), module, name)
}

fn relation(schema: &Schema, module: &str, name: &str) -> RelId {
    let m = schema.module_named(module).expect("the module");
    match schema.module(m).values.get(name) {
        Some(ValueDef::Relation(r)) => *r,
        _ => panic!("no relation `{module}::{name}`"),
    }
}

/// Returns the position of field `field` in a row of `catalog::indexes`.
fn index_field(catalog: &Catalog, field: &str) -> usize {
    layout::row(catalog.schema(), rel(catalog, "catalog", "indexes"), 0)
        .iter()
        .position(|(n, _)| n == field)
        .expect("an Index row has the field")
}

/// Returns each row of `catalog::indexes` over `app::items` other than its key's.
fn declared_rows(
    pager: &Pager<SimRuntime>,
    catalog: &Catalog,
    txns: &Transactions<SimRuntime>,
) -> Vec<(u32, Vec<Value>)> {
    let items = catalog
        .schema()
        .ids()
        .relation_id(rel(catalog, "app", "items"))
        .expect("items has an id")
        .raw();
    let (relation, clause) = (
        index_field(catalog, "relation"),
        index_field(catalog, "clause"),
    );
    let hold = txns.take(Scope::Every).expect("a snapshot");
    let rows = catalog
        .rows(
            &Snapshot::new(pager, txns, (&hold, None)),
            rel(catalog, "catalog", "indexes"),
        )
        .expect("the rows read");
    drop(hold);
    rows.into_iter()
        .filter(|(_, row)| row[relation] == Value::I64(items as i64))
        .filter(|(_, row)| row[clause] != Value::Text("key id".into()))
        .collect()
}

/// Returns the clause of each row of `catalog::indexes` over `app::items` other than its key's.
fn clauses(
    pager: &Pager<SimRuntime>,
    catalog: &Catalog,
    txns: &Transactions<SimRuntime>,
) -> Vec<String> {
    let clause = index_field(catalog, "clause");
    declared_rows(pager, catalog, txns)
        .into_iter()
        .map(|(_, row)| match &row[clause] {
            Value::Text(t) => t.clone(),
            v => panic!("a clause is text: {v:?}"),
        })
        .collect()
}

/// Replace the clause of the one declared index of `app::items` with `clause`, as an older
/// engine stored it, in a transaction of its own.
fn restate(
    pager: &Pager<SimRuntime>,
    catalog: &Catalog,
    txns: &Transactions<SimRuntime>,
    clause: &str,
) {
    let [(c, row)]: [(u32, Vec<Value>); 1] = declared_rows(pager, catalog, txns)
        .try_into()
        .expect("one declared index");
    let indexes = rel(catalog, "catalog", "indexes");
    let begun = txns.begin(pager).expect("a transaction");
    let hold = txns.take(Scope::Every).expect("a snapshot");
    let m = Migrating {
        pager,
        txns,
        x: begun.xid(),
        hold,
        made: Default::default(),
        top: Default::default(),
    };
    delete_rows(&m, catalog.heap(indexes, c), std::slice::from_ref(&row)).expect("deleted");
    let mut older = row;
    older[index_field(catalog, "clause")] = Value::Text(clause.into());
    catalog.insert(&m, indexes, c, &older).expect("inserted");
    drop(m);
    txns.commit(pager, begun).expect("committed").done();
}

const LOWER: &str = "module app
add module app
add type Item = { id: Key items, name: Text }
add relation items : Item key id generate sequence
  index (lower(name))
";

/// Returns the rows of `catalog::indexes` over the relation whose catalog id is `relation`.
fn index_rows_over(
    pager: &Pager<SimRuntime>,
    catalog: &Catalog,
    txns: &Transactions<SimRuntime>,
    relation: i64,
) -> Vec<(u32, Vec<Value>)> {
    let at = index_field(catalog, "relation");
    let hold = txns.take(Scope::Every).expect("a snapshot");
    let rows = catalog
        .rows(
            &Snapshot::new(pager, txns, (&hold, None)),
            rel(catalog, "catalog", "indexes"),
        )
        .expect("the rows read");
    drop(hold);
    rows.into_iter()
        .filter(|(_, row)| row[at] == Value::I64(relation))
        .collect()
}

#[test]
fn a_migration_drops_an_index_whose_clause_an_older_engine_stored_otherwise() {
    let mut sim = Sim::new(40, Config::default());
    let mut ran = false;
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options(64)).expect("create");
        let (mut catalog, txns) = Catalog::create(rt, &pager).expect("the catalog");
        catalog
            .migrate(&pager, &txns, LOWER)
            .expect("the index applies");
        assert_eq!(
            vec!["index (lower ( name ))"],
            clauses(&pager, &catalog, &txns)
        );
        restate(&pager, &catalog, &txns, "index (lower(name))");
        assert_eq!(
            vec!["index (lower(name))"],
            clauses(&pager, &catalog, &txns),
            "setup: the older clause is stored"
        );
        let items = catalog
            .schema()
            .ids()
            .relation_id(rel(&catalog, "app", "items"))
            .expect("items has an id")
            .raw() as i64;
        assert_eq!(2, index_rows_over(&pager, &catalog, &txns, items).len());
        catalog
            .migrate(
                &pager,
                &txns,
                "module app\ndrop relation items\ndrop type Item\n",
            )
            .expect("the drop applies");
        assert_eq!(
            Vec::<(u32, Vec<Value>)>::new(),
            index_rows_over(&pager, &catalog, &txns, items)
        );
        ran = true;
    });
    assert!(ran);
}

/// Returns `source` migrated by `text`.
fn migrated(source: &Source, text: &str) -> Source {
    let file = parse_migration(text, Options::default())
        .ok()
        .expect("the migration parses");
    source.migrate(&file, text).expect("the migration applies")
}

#[test]
fn a_migration_that_only_moves_an_index_declaration_rewrites_no_relation() {
    let before = migrated(
        &Source::default(),
        &declaring("match s | Draft -> 0 | Sent t -> t.at"),
    );
    let after = migrated(
        &before,
        "module app\nrename relation items to stock_items\n",
    );
    let old_compiled = before.check().expect("the source checks");
    let old = old_compiled.schema();
    let new_compiled = after.check().expect("the source checks");
    let new = new_compiled.schema();
    let span = |schema: &Schema, name: &str| schema.relation(relation(schema, "app", name)).span;
    assert_ne!(
        span(old, "items"),
        span(new, "stock_items"),
        "the declaration moves"
    );
    assert_eq!(
        BTreeSet::new(),
        change::rewritten(
            old,
            new,
            &adtb_lang::depends::changed(&old_compiled, &new_compiled),
        )
    );
}
