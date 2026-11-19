use std::collections::BTreeSet;

use adtb_lang::ast::{CallArgs, ExprKind, Lit};
use adtb_lang::evolve::Source;
use adtb_lang::parse::{Options, parse_expr, parse_migration};
use adtb_lang::schema::{Schema, ValueDef};
use adtb_lang::types::RelId;
use adtb_lang::visit::each_expr;
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_testkit::search::Files;

use crate::catalog::{Catalog, Error, Migrating, change, delete_rows, layout};
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

/// What a migration declaring an index over one element does.
#[derive(Debug, PartialEq, Eq)]
enum Declared {
    /// The catalog stores the element as this text.
    Stored(String),
    /// The migration is refused; each reason.
    Refused(Vec<String>),
}

use Declared::{Refused, Stored};

fn stored(element: &str) -> Declared {
    Stored(element.to_string())
}

fn refused(why: &[&str]) -> Declared {
    Refused(why.iter().map(ToString::to_string).collect())
}

fn not_admitted(element: &str, why: &str) -> Declared {
    refused(&[&format!("index `index ({element})` is not admitted: {why}")])
}

/// Returns each index element, another spelling of it, and what declaring it does: the
/// canonical text of each element the catalog stores, by the token rules of canonical source text,
/// and each refusal.
fn elements() -> Vec<(&'static str, Option<&'static str>, Declared)> {
    let unscoped = "no `items` in scope";
    let checker = |why: &'static str| refused(&[why, unscoped, unscoped]);
    vec![
        ("n + 007", Some("n+7"), stored("n + 7")),
        ("d + 01.50", Some("d+1.50"), stored("d + 1.50")),
        ("f + 1.50e03", Some("f+1.50e3"), stored("f + 1.50e3")),
        (
            "name ++ \"a\\\"b\\\\c\\td\"",
            Some("name++\"a\\\"b\\\\c\td\""),
            stored("name ++ \"a\\\"b\\\\c\\td\""),
        ),
        ("when + 090s", Some("when+90s"), stored("when + 90s")),
        (
            "n > 0 or true",
            Some("n>0 or true"),
            stored("n > 0 or true"),
        ),
        (
            "match s | Draft -> 0 | Sent t -> t.at",
            Some("match s\n    | Draft->0\n    | Sent t->t . at"),
            stored("match s | app::Draft -> 0 | app::Sent t -> t . at"),
        ),
        (
            "app::twice(n)",
            Some("twice( n )"),
            stored("app::twice ( n )"),
        ),
        (
            "apply(fn y -> y > 0, n)",
            Some("app::apply(fn y->y>0,n)"),
            stored("app::apply ( fn y -> y > 0 , n )"),
        ),
        (
            "Instant.parse(name)",
            Some("Instant . parse(name)"),
            stored("Instant . parse ( name )"),
        ),
        (
            "Sent { at: n }",
            Some("Sent{at:n}"),
            stored("app::Sent { at : n }"),
        ),
        ("Draft", Some("  Draft  "), stored("app::Draft")),
        (
            "{ a: name, b: n }",
            Some("{a:name,b:n}"),
            stored("{ a : name , b : n }"),
        ),
        (
            "if () == () then n else 0",
            Some("if ( ) == ( ) then n else 00"),
            stored("if ( ) == ( ) then n else 0"),
        ),
        ("-n", Some("- n"), stored("- n")),
        ("{ a: n }.a", Some("{a: n} . a"), stored("{ a : n } . a")),
        (
            "items.key(n)",
            Some("items .key( n )"),
            stored("app::items . key ( n )"),
        ),
        (
            "if n > 0 then name else \"none\"",
            Some("if n>0 then name else \"none\""),
            stored("if n > 0 then name else \"none\""),
        ),
        (
            "count(x for x in items where x.n == 1)",
            None,
            not_admitted(
                "count ( x for x in app::items where x . n == 1 )",
                "an aggregate in a scalar expression",
            ),
        ),
        (
            "exists x in [n] where x > 0",
            None,
            not_admitted(
                "exists x in [ n ] where x > 0",
                "an `exists` in a scalar expression",
            ),
        ),
        (
            "exists t in id.<tags.item where t.n > 0",
            None,
            not_admitted(
                "exists t in id .< app::tags . item where t . n > 0",
                "an `exists` in a scalar expression",
            ),
        ),
        (
            "exists x in (from y in [n] select y) where x == n",
            None,
            not_admitted(
                "exists x in ( from y in [ n ] select y ) where x == n",
                "an `exists` in a scalar expression",
            ),
        ),
        (
            "exists x in (fix a = [n] in a) where x == n",
            None,
            not_admitted(
                "exists x in ( fix a = [ n ] in a ) where x == n",
                "an `exists` in a scalar expression",
            ),
        ),
        (
            "(if n > 0 then s else Draft).Sent",
            None,
            checker("`.C` selects constructors of a set; a single value is examined by `match`"),
        ),
        (
            "exists x in (top 1 by (y) (from y in [n] select y)) where x == n",
            None,
            checker(
                "`top` needs a determinate order: include the row's key, or a field a key or unique covers",
            ),
        ),
        (
            "at instant when { n }",
            None,
            checker("an `at` block reads a set, found I64"),
        ),
        (
            "if n > 0 then n else reject \"no\"",
            None,
            checker("`reject` is allowed only in the source query of an insert"),
        ),
        (
            "(insert items { name: \"a\", n: 1, f: 1.0e0, d: 1.00, s: Draft, when: when })",
            None,
            checker(
                "a write appears only where a change set is expected: in a transact block, a script or ad hoc text",
            ),
        ),
    ]
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
        .rows(pager, txns, &hold, rel(catalog, "catalog", "indexes"))
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

/// Returns the files of a closed file holding a fresh catalog.
fn created() -> Files {
    let mut sim = Sim::new(0, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options(64)).expect("create");
        Catalog::create(rt, &pager).expect("the catalog");
        pager.close();
    });
    sim.files()
}

/// Returns what the fresh catalog of `files` does with a migration declaring an index over
/// `element`.
fn declare(files: &Files, seed: u64, element: &str) -> Declared {
    let mut sim = Sim::with_files(seed, Config::default(), files.clone());
    let mut out = None;
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options(64)).expect("the file opens");
        let (mut catalog, txns) = Catalog::open(rt, &pager).expect("the catalog");
        out = Some(match catalog.migrate(&pager, &txns, &declaring(element)) {
            Ok(()) => {
                let declared = clauses(&pager, &catalog, &txns);
                let [clause] = declared.as_slice() else {
                    panic!("one declared index: {declared:?}");
                };
                let element = clause
                    .strip_prefix("index (")
                    .and_then(|c| c.strip_suffix(')'))
                    .unwrap_or_else(|| panic!("`{clause}` is an index clause"));
                stored(element)
            }
            Err(Error::Refused(why)) => Refused(why),
            Err(e) => panic!("the migration applies or is refused: {e:?}"),
        });
    });
    out.expect("the simulation ran")
}

#[test]
fn every_index_element_form_is_stored_as_one_canonical_text_whatever_its_spelling() {
    let mut forms = BTreeSet::new();
    let mut respelled = 0;
    let files = created();
    for (seed, (element, other, want)) in elements().into_iter().enumerate() {
        let seed = seed as u64;
        forms.extend(forms_of(element));
        assert_eq!(
            want,
            declare(&files, seed, element),
            "`{element}`, seed {seed}"
        );
        if let Some(other) = other {
            assert_eq!(want, declare(&files, seed, other), "`{other}`, seed {seed}");
            respelled += 1;
        }
    }
    assert_eq!(18, respelled);
    assert_eq!(BTreeSet::from(FORMS), forms);
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
        hold: &hold,
        made: Default::default(),
        top: Default::default(),
    };
    delete_rows(&m, catalog.heap(indexes, c), std::slice::from_ref(&row)).expect("deleted");
    let mut older = row;
    older[index_field(catalog, "clause")] = Value::Text(clause.into());
    catalog.insert(&m, indexes, c, &older).expect("inserted");
    drop(hold);
    txns.commit(pager, begun).expect("committed");
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
        .rows(pager, txns, &hold, rel(catalog, "catalog", "indexes"))
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
    let (old, old_program) = before.check().expect("the source checks");
    let (new, new_program) = after.check().expect("the source checks");
    let span = |schema: &Schema, name: &str| schema.relation(relation(schema, "app", name)).span;
    assert_ne!(
        span(&old, "items"),
        span(&new, "stock_items"),
        "the declaration moves"
    );
    assert_eq!(
        BTreeSet::new(),
        change::rewritten(&old, &old_program, &new, &new_program)
    );
}
