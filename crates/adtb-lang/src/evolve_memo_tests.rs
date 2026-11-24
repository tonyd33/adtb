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

fn built_uncached(src: &Source) -> Result<Arc<Schema>, Vec<FileDiag>> {
    Ok(src
        .inputs()
        .build(&memo::ParsedMemo::default())?
        .schema()
        .clone())
}

/// Assert that `src` builds what an uncached build gives, on a miss or hit and on the hit after.
fn builds_uncached(src: &Source) -> String {
    let uncached = printed(&built_uncached(src).unwrap());
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

/// Returns `src` after `text`, every hit of a memo its migration makes checked against an
/// uncached build.
fn apply_checked(src: &Source, text: &str) -> Source {
    crate::sampled::checking_every_hit(|| apply(src, text))
}

#[test]
fn a_memoised_build_equals_an_uncached_one_across_a_migration_walk() {
    let mut src = base();
    let mut cold = base();
    builds_uncached(&src);
    let checked = || {
        (
            crate::sampled::BUILDS.checked(),
            crate::sampled::MODEL_BUILDS.checked(),
        )
    };
    let before = checked();
    for (i, text) in WALK.iter().enumerate() {
        expecting_alone_is_rebuilt(&src, text);
        src = apply_checked(&src, text);
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
    let after = checked();
    assert!(
        after.0 > before.0 && after.1 > before.1,
        "the walk's migrations hit a source's memo and a model's: {before:?} then {after:?}"
    );
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
    let ahead = |typed| {
        Expected::new(
            "app".to_string(),
            "later".to_string(),
            typed,
            Adder::Operator(0),
        )
    };
    changed.ahead.push(ahead(false));
    is_rebuilt(&src, &changed);
    let mut typed = src;
    typed.ahead.push(ahead(true));
    assert_ne!(changed.ahead, typed.ahead);
    builds_uncached(&typed);
}

#[test]
fn a_declaration_that_only_ahead_lets_resolve_refuses_without_it() {
    let mut src = base();
    src.decls[1].text = "type Note = { later: Key later }".to_string();
    let text = src.file_text(FileId(1));
    let at = text.rfind("later").unwrap();
    let unresolved = vec![FileDiag {
        file: FileId(1),
        diag: Diag::new(Span::new(at, text.len() - 2), "no `later` in scope"),
    }];
    assert_eq!(Err(unresolved.clone()), src.built().map(|_| ()));
    let mut ahead = src.clone();
    ahead.ahead.push(Expected::new(
        "other".to_string(),
        "later".to_string(),
        false,
        Adder::Operator(0),
    ));
    builds_uncached(&ahead);
    assert_eq!(Err(unresolved), src.built().map(|_| ()));
}

#[test]
fn a_failed_build_does_not_disturb_the_memo() {
    let src = base();
    let held = builds_uncached(&src);
    let mut broken = src.clone();
    broken.decls[1].text = "type".to_string();
    let end = broken.file_text(FileId(1)).len();
    let unparsed = Err(vec![FileDiag {
        file: FileId(1),
        diag: Diag::new(
            Span::new(end, end),
            "expected a type name, found the end of the text",
        ),
    }]);
    assert_eq!(unparsed, broken.built().map(|_| ()));
    assert_eq!(unparsed, broken.built().map(|_| ()));
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
    assert!(src.memo.get(&src.inputs()).is_some());
    assert!(src.memo.get(&first.inputs()).is_none());
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
    let held = src.memo.get(&src.inputs()).unwrap().schema().clone();
    let entries = src.memo.len();
    src.built().unwrap();
    assert!(Arc::ptr_eq(
        &held,
        src.memo.get(&src.inputs()).unwrap().schema()
    ));
    assert_eq!(entries, src.memo.len());
}

#[test]
fn two_schemas_of_a_source_share_their_declarations() {
    let src = base();
    let first = src.schema().unwrap();
    let second = src.schema().unwrap();
    assert!(std::ptr::eq(
        std::ops::Deref::deref(&first),
        std::ops::Deref::deref(&second)
    ));
}
