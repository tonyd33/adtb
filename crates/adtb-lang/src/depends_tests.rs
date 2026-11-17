use std::collections::BTreeSet;

use super::{bodies, changed, reached};
use crate::check::Program;
use crate::evolve::Source;
use crate::norm::Root;
use crate::parse::{Options, parse_migration};
use crate::schema::{Schema, ValueDef};
use crate::types::FnId;

/// A schema in which `f` is reached from a body of every kind a program holds but a retag, whose
/// payload is literals.
const EVERY_KIND: &str = r"module app
add module app
define fn f (x: I64) -> I64 = x * 2
define fn g (x: I64) -> I64 = f(x)
define aggregate total (x: I64) -> I64 = state { n: I64 } init { n: f(0) } step (s, x) -> { n: s.n + x } merge (a, b) -> { n: a.n + b.n } finish s -> s.n
add type Small = new I64 where f(self) < 1000
add type User = { id: Key users, n: I64 }
add relation users : User key id generate sequence
add type Item = { id: Key items, n: I64, s: Small }
add relation items : Item key id generate sequence
  index (f(n))
add default items.n = f(1)
add computed items.c: I64 = f(self.n)
add check bounded on items (g(n) < 100)
add constraint few =
  from i in items where total(v for v in [i.n]) == Some { value: 100 } select i
";

/// Returns `source` migrated by `text`.
fn migrated(source: &Source, text: &str) -> Source {
    let file = parse_migration(text, Options::default())
        .ok()
        .expect("the migration parses");
    source
        .migrate(&file, text)
        .unwrap_or_else(|e| panic!("the migration applies: {e:?}"))
}

fn checked(source: &Source) -> (Schema, Program) {
    source
        .check()
        .unwrap_or_else(|ds| panic!("the source checks: {ds:?}"))
}

fn function(schema: &Schema, name: &str) -> FnId {
    let m = schema.module_named("app").expect("app");
    match schema.module(m).values.get(name) {
        Some(ValueDef::Function(f)) => *f,
        _ => panic!("no function `{name}`"),
    }
}

/// Returns the name of the kind of `root`; `None` for a body of ad hoc text.
fn kind(root: Root) -> Option<&'static str> {
    Some(match root {
        Root::Function(_) => "function",
        Root::Constraint(_) => "constraint",
        Root::Check(_) => "check",
        Root::Domain(_) => "domain",
        Root::Retag(_) => "retag",
        Root::Default(..) => "default",
        Root::Computed(..) => "computed",
        Root::IndexField(..) => "index field",
        Root::Aggregate(_) => "aggregate",
        Root::Item(_) | Root::Command(_) | Root::Query => return None,
    })
}

/// Returns whether `root` is the body of an object of module `app`.
fn of_app(schema: &Schema, root: Root) -> bool {
    let module = match root {
        Root::Function(f) => schema.function(f).module,
        Root::Constraint(c) => schema.constraint(c).module,
        Root::Check(c) => schema.check(c).module,
        Root::Domain(t) => schema.ty(t).module,
        Root::Retag(r) | Root::Default(r, _) | Root::Computed(r, _) | Root::IndexField(r, ..) => {
            schema.relation(r).module
        }
        Root::Aggregate(i) => schema.aggregates[i].module,
        Root::Item(_) | Root::Command(_) | Root::Query => return false,
    };
    Some(module) == schema.module_named("app")
}

fn every_kind() -> Source {
    migrated(&Source::default(), EVERY_KIND)
}

#[test]
fn every_kind_of_body_reaches_the_functions_it_calls() {
    let source = every_kind();
    let (schema, program) = checked(&source);
    let f = Root::Function(function(&schema, "f"));
    let reached = reached(&program);
    let mut kinds = BTreeSet::new();
    for (root, _, _) in bodies(&program) {
        if root == f || !of_app(&schema, root) {
            continue;
        }
        assert!(
            reached[&root].contains(&f),
            "{root:?} reaches {:?}",
            reached[&root]
        );
        kinds.extend(kind(root));
    }
    let all = BTreeSet::from([
        "function",
        "constraint",
        "check",
        "domain",
        "default",
        "computed",
        "index field",
        "aggregate",
    ]);
    assert_eq!(all, kinds);
}

#[test]
fn a_redefinition_changes_every_body_that_reaches_it_and_only_when_its_body_changes() {
    let before = every_kind();
    let (a, old) = checked(&before);
    let same = migrated(
        &before,
        "module app\nredefine fn f (x: I64) -> I64 = x * 2\n",
    );
    let (b, new) = checked(&same);
    assert_eq!(BTreeSet::new(), changed(&a, &old, &b, &new));
    let other = migrated(
        &before,
        "module app\nredefine fn f (x: I64) -> I64 = x + 2\n",
    );
    let (b, new) = checked(&other);
    let every: BTreeSet<Root> = bodies(&new)
        .into_iter()
        .map(|(r, _, _)| r)
        .filter(|&r| of_app(&b, r))
        .collect();
    assert_eq!(every, changed(&a, &old, &b, &new));
}

#[test]
fn a_function_named_at_a_reused_position_is_a_change() {
    let before = migrated(
        &Source::default(),
        "module app
add module app
define fn a (x: I64) -> I64 = x
define fn g (x: I64) -> I64 = x + 1
define fn h (x: I64) -> I64 = x + 2
define fn f (x: I64) -> I64 = g(x)
add type Item = { id: Key items, n: I64 }
add relation items : Item key id generate sequence
  index (f(n))
",
    );
    let (a, old) = checked(&before);
    let after = migrated(
        &before,
        "module app\nredefine fn f (x: I64) -> I64 = h(x)\ndrop fn g\n",
    );
    let (b, new) = checked(&after);
    assert_eq!(
        FnId(1),
        function(&b, "h"),
        "setup: `h` takes the position of `g`"
    );
    let index = bodies(&new)
        .into_iter()
        .map(|(r, _, _)| r)
        .filter(|&r| matches!(r, Root::IndexField(..)) && of_app(&b, r))
        .collect::<Vec<_>>();
    let [index] = index.as_slice() else {
        panic!("one index element of `app`: {index:?}");
    };
    let want = BTreeSet::from([Root::Function(function(&b, "f")), *index]);
    assert_eq!(want, changed(&a, &old, &b, &new));
}

#[test]
fn a_shift_of_positions_changes_no_body() {
    let before = migrated(
        &Source::default(),
        "module app
add module app
define fn a (x: I64) -> I64 = x
define fn f (x: I64) -> I64 = x * 2
add type Pre = new I64
add type Item = { id: Key items, n: I64 }
add relation items : Item key id generate sequence
  index (f(n))
add check bounded on items (f(n) < 100)
add constraint big =
  from i in items where f(i.n) > 100 select i
",
    );
    let (a, old) = checked(&before);
    let shifts = [
        ("module app\ndrop fn a\n", None),
        (
            "module app\ndrop fn a\ndefine fn a (x: I64) -> I64 = x\n",
            Some("a"),
        ),
        ("module app\ndrop type Pre\n", None),
    ];
    for (text, added) in shifts {
        let after = migrated(&before, text);
        let (b, new) = checked(&after);
        let item = |s: &Schema| s.module(s.module_named("app").expect("app")).types["Item"];
        assert!(
            function(&a, "f") != function(&b, "f") || item(&a) != item(&b),
            "setup: `{text}` shifts a position"
        );
        let want: BTreeSet<Root> = added
            .map(|n| Root::Function(function(&b, n)))
            .into_iter()
            .collect();
        assert_eq!(want, changed(&a, &old, &b, &new), "{text}");
    }
}
