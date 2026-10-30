use super::*;
use crate::parse::parse_migration;

const BASE: &str = "type User = { id: Key users, name: Text }
relation users : User key id generate sequence";

fn base() -> Source {
    let mut src = Source::new(vec![
        SourceDecl {
            module: "app".to_string(),
            text: BASE.to_string(),
        },
        SourceDecl {
            module: "other".to_string(),
            text: "type Note = { text: Text }".to_string(),
        },
    ])
    .unwrap();
    src.modules.insert("spare".to_string());
    src
}

fn printed(schema: &Schema) -> String {
    format!("{schema:?}")
}

/// Assert that `src` builds what an uncached build gives, on a miss or hit and on the hit after.
fn builds_uncached(src: &Source) -> String {
    let uncached = printed(&src.built_uncached().unwrap());
    assert_eq!(uncached, printed(&src.built().unwrap()));
    assert_eq!(uncached, printed(&src.built().unwrap()));
    uncached
}

/// Assert that `changed`, sharing the memo of `src` after `src` built, builds what an uncached
/// build gives and that this differs from what `src` builds.
fn is_rebuilt(src: &Source, changed: &Source) {
    assert_ne!(builds_uncached(src), builds_uncached(changed));
}

fn apply(src: &Source, text: &str) -> Source {
    let m = parse_migration(text, Options::default()).ok().unwrap();
    src.migrate(&m, text).unwrap()
}

const WALK: [&str; 7] = [
    "module app\nadd field User.age: I64 fill 0",
    "module app\nadd type Team = { id: Key teams, lead: Key members }
add type Member = { id: Key members, team: Key teams }
add relation teams : Team key id generate sequence index (lead)
add relation members : Member key id generate sequence index (team)",
    "module app\nrename field User.age to years",
    "module empty\nadd module empty",
    "module empty\nadd type Label = { id: Key tags }
add relation tags : Label key id generate sequence",
    "module empty\ndrop relation tags\ndrop type Label",
    "module app\ndrop field User.years",
];

/// Assert that a step whose operators add a relation changes the build by `ahead` alone.
fn expecting_alone_is_rebuilt(src: &Source, text: &str) {
    let m = parse_migration(text, Options::default()).ok().unwrap();
    let mut expecting = src.clone();
    expecting.expecting(&m.module.text, &m.operators);
    if !expecting.ahead.is_empty() {
        is_rebuilt(src, &expecting);
    }
}

#[test]
fn a_memoised_build_equals_an_uncached_one_across_a_migration_walk() {
    let mut src = base();
    let mut cold = base();
    builds_uncached(&src);
    for (i, text) in WALK.iter().enumerate() {
        expecting_alone_is_rebuilt(&src, text);
        src = apply(&src, text);
        cold.memo = memo::BuiltMemo::default();
        cold = apply(&cold, text);
        assert_eq!(
            builds_uncached(&src),
            builds_uncached(&cold),
            "after step {i}"
        );
        assert_eq!(
            printed(&src.schema().unwrap()),
            printed(&cold.schema().unwrap()),
            "after step {i}"
        );
    }
}

#[test]
fn a_change_of_declaration_text_alone_is_rebuilt() {
    let src = base();
    let mut changed = src.clone();
    changed.decls[1]
        .text
        .push_str("\ntype Extra = { text: Text }");
    is_rebuilt(&src, &changed);
}

#[test]
fn a_change_of_a_declarations_module_alone_is_rebuilt() {
    let src = base();
    let mut changed = src.clone();
    changed.decls[1].module = "spare".to_string();
    changed.modules.insert("other".to_string());
    assert_eq!(src.modules, changed.modules);
    is_rebuilt(&src, &changed);
}

#[test]
fn a_change_of_the_empty_modules_alone_is_rebuilt() {
    let src = base();
    let mut changed = src.clone();
    changed.modules.insert("empty".to_string());
    is_rebuilt(&src, &changed);
}

#[test]
fn a_change_of_the_relations_ahead_alone_is_rebuilt() {
    let src = base();
    let mut changed = src.clone();
    let ahead = schema::Ahead {
        module: "app".to_string(),
        relation: "later".to_string(),
        typed: false,
    };
    changed.ahead.push(ahead);
    is_rebuilt(&src, &changed);
    let mut typed = changed.clone();
    typed.ahead[0].typed = true;
    assert_ne!(changed.ahead, typed.ahead);
    builds_uncached(&typed);
}

#[test]
fn a_declaration_that_only_ahead_lets_resolve_refuses_without_it() {
    let mut src = base();
    src.decls[1].text = "type Note = { later: Key later }".to_string();
    assert!(src.built().is_err());
    let mut ahead = src.clone();
    ahead.ahead.push(schema::Ahead {
        module: "other".to_string(),
        relation: "later".to_string(),
        typed: false,
    });
    builds_uncached(&ahead);
    assert!(src.built().is_err());
}

#[test]
fn a_failed_build_does_not_disturb_the_memo() {
    let src = base();
    let held = builds_uncached(&src);
    let mut broken = src.clone();
    broken.decls[1].text = "type".to_string();
    assert!(broken.built().is_err());
    assert!(broken.built().is_err());
    assert_eq!(held, builds_uncached(&src));
}

fn numbered(src: &Source, n: usize) -> Source {
    let mut out = src.clone();
    out.modules.insert(format!("spare{n}"));
    out
}

#[test]
fn the_memo_keeps_the_most_recently_used_builds_up_to_its_bound() {
    let src = base();
    src.built().unwrap();
    let first = numbered(&src, 0);
    first.built().unwrap();
    for n in 1..memo::BOUND - 1 {
        numbered(&src, n).built().unwrap();
    }
    assert_eq!(memo::BOUND, src.memo.len());
    src.built().unwrap();
    numbered(&src, memo::BOUND).built().unwrap();
    assert_eq!(memo::BOUND, src.memo.len());
    assert!(src.memo.get(&src).is_some());
    assert!(src.memo.get(&first).is_none());
    builds_uncached(&first);
    assert_eq!(memo::BOUND, src.memo.len());
}

#[test]
fn the_memo_does_not_show_in_a_sources_debug_output() {
    let src = Source::default();
    let cold = format!("{src:?}");
    src.built().unwrap();
    assert_eq!(cold, format!("{src:?}"));
}

#[test]
fn a_second_build_of_a_source_is_served_from_the_memo() {
    let src = base();
    src.built().unwrap();
    let held = src.memo.get(&src).unwrap();
    let entries = src.memo.len();
    src.built().unwrap();
    assert!(Arc::ptr_eq(&held, &src.memo.get(&src).unwrap()));
    assert_eq!(entries, src.memo.len());
}
