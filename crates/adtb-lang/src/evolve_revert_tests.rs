use super::*;

const BASE: &str = r"
module auth
type U = { id: Key us, name: Text }
relation us : U key id generate sequence
type T { id: Key ts } = A { x: Text } | B { y: Text }
relation ts : T key id generate sequence
";

/// A migration that names no object a case's first migration creates.
const NEUTRAL: &str = "rename field U.name to label";

/// One object a first migration creates or changes, and migrations that rename it, rename the
/// object its name is spelled under, put a distinct object of its old spelling in place, and
/// name it. Templates spell the object `{X}` and its parent `{P}` by their current names.
struct Case {
    what: &'static str,
    first: &'static str,
    /// Whether `first` creates the object rather than changing one it keeps the id of.
    created: bool,
    parent: &'static str,
    object: &'static str,
    rename: Option<(&'static str, &'static str)>,
    rename_parent: Option<(&'static str, &'static str)>,
    /// Applied after `rename_parent`, with the old spellings.
    decoy: Option<&'static str>,
    names: &'static str,
    /// Names the decoy by the old spellings.
    names_decoy: Option<&'static str>,
    shown: fn(&str, &str) -> String,
}

const CASES: &[Case] = &[
    Case {
        what: "a constructor named as an operand",
        first: "add constructor T.C { z: Text }",
        created: true,
        parent: "T",
        object: "C",
        rename: Some(("rename constructor {P}.{X} to D", "D")),
        rename_parent: Some(("rename type {P} to T2", "T2")),
        decoy: Some("add type T = C | E"),
        names: "enable ts.{X}",
        names_decoy: Some("rename constructor T.C to G"),
        shown: |p, x| format!("constructor `auth::{p}.{x}`"),
    },
    Case {
        what: "a constructor named in a body",
        first: "add constructor T.C { z: Text }",
        created: true,
        parent: "T",
        object: "C",
        rename: Some(("rename constructor {P}.{X} to D", "D")),
        rename_parent: Some(("rename type {P} to T2", "T2")),
        decoy: Some("add type T = C | E"),
        names: "define fn h (t: {P}) -> Bool = match t | A a -> false | B b -> false | {X} c -> true",
        names_decoy: Some("define fn h (t: T) -> Bool = match t | C c -> true | E e -> false"),
        shown: |p, x| format!("constructor `auth::{p}.{x}`"),
    },
    Case {
        what: "a field named as an operand",
        first: "add field T.A.z: Text fill \"a\"",
        created: true,
        parent: "A",
        object: "z",
        rename: Some(("rename field T.{P}.{X} to z2", "z2")),
        rename_parent: Some(("rename constructor T.{P} to A2", "A2")),
        decoy: Some("add constructor T.A { z: Text }"),
        names: "rename field T.{P}.{X} to w",
        names_decoy: Some("rename field T.A.z to w"),
        shown: |p, x| format!("field `auth::T.{p}.{x}`"),
    },
    Case {
        what: "a field named in a body",
        first: "add field T.A.z: Text fill \"a\"",
        created: true,
        parent: "A",
        object: "z",
        rename: Some(("rename field T.{P}.{X} to z2", "z2")),
        rename_parent: Some(("rename constructor T.{P} to A2", "A2")),
        decoy: None,
        names: "define fn h (t: T) -> Text = match t | {P} a -> a.{X} | B b -> b.y",
        names_decoy: None,
        shown: |p, x| format!("field `auth::T.{p}.{x}`"),
    },
    Case {
        what: "the constructor of a field added to it",
        first: "add constructor T.C { z: Text }",
        created: true,
        parent: "T",
        object: "C",
        rename: None,
        rename_parent: Some(("rename type {P} to T2", "T2")),
        decoy: Some("add type T = C { v: Text } | E"),
        names: "add field {P}.{X}.w: Text fill \"w\"",
        names_decoy: Some("add field T.C.w: Text fill \"w\""),
        shown: |p, x| format!("constructor `auth::{p}.{x}`"),
    },
    Case {
        what: "a set",
        first: "add set S of ts = A",
        created: true,
        parent: "ts",
        object: "S",
        rename: Some(("rename set {X} of {P} to S1", "S1")),
        rename_parent: Some(("rename relation {P} to ts2", "ts2")),
        decoy: None,
        names: "rename set {X} of {P} to S2",
        names_decoy: None,
        shown: |p, x| format!("set `auth::{p}.{x}`"),
    },
    Case {
        what: "a type named as an operand",
        first: "add type P = P1 | P2",
        created: true,
        parent: "",
        object: "P",
        rename: Some(("rename type {X} to Q", "Q")),
        rename_parent: None,
        decoy: None,
        names: "add constructor {X}.P3",
        names_decoy: None,
        shown: |_, x| format!("type `auth::{x}`"),
    },
    Case {
        what: "a type named in a declaration",
        first: "add type P = P1 | P2",
        created: true,
        parent: "",
        object: "P",
        rename: Some(("rename type {X} to Q", "Q")),
        rename_parent: None,
        decoy: None,
        names: "add type W = { p: {X} }",
        names_decoy: None,
        shown: |_, x| format!("type `auth::{x}`"),
    },
    Case {
        what: "a relation named in a declaration",
        first: "add type V = { id: Key vs }\nadd relation vs : V key id generate sequence",
        created: true,
        parent: "",
        object: "vs",
        rename: Some(("rename relation {X} to ws", "ws")),
        rename_parent: None,
        decoy: None,
        names: "add type W = { v: Key {X} }",
        names_decoy: None,
        shown: |_, x| format!("relation `auth::{x}`"),
    },
    Case {
        what: "a function named in a body",
        first: "define fn f (u: U) -> Text = u.name",
        created: true,
        parent: "",
        object: "f",
        rename: None,
        rename_parent: None,
        decoy: None,
        names: "define fn g (u: U) -> Text = {X}(u)",
        names_decoy: None,
        shown: |_, x| format!("function `auth::{x}`"),
    },
    Case {
        what: "a field named in a grant's field list",
        first: "add field T.A.z: Text fill \"a\"",
        created: true,
        parent: "A",
        object: "z",
        rename: Some(("rename field T.{P}.{X} to z2", "z2")),
        rename_parent: Some(("rename constructor T.{P} to A2", "A2")),
        decoy: Some("add constructor T.A { z: Text }"),
        names: "grant read ts.{P} ({X}) to support",
        names_decoy: Some("grant read ts.A (z) to support"),
        shown: |p, x| format!("field `auth::T.{p}.{x}`"),
    },
    Case {
        what: "a field named qualified in a grant's field list",
        first: "add field T.B.x: Text fill \"b\"",
        created: true,
        parent: "B",
        object: "x",
        rename: Some(("rename field T.{P}.{X} to x2", "x2")),
        rename_parent: Some(("rename constructor T.{P} to B2", "B2")),
        decoy: Some("add constructor T.B { x: Text }"),
        names: "grant read ts (A.x, {P}.{X}) to support",
        names_decoy: Some("grant read ts (A.x, B.x) to support"),
        shown: |p, x| format!("field `auth::T.{p}.{x}`"),
    },
    Case {
        what: "a module a later migration declares in",
        first: "module m\nadd module m",
        created: true,
        parent: "",
        object: "m",
        rename: None,
        rename_parent: None,
        decoy: None,
        names: "module {X}\nadd type X = X1 | X2",
        names_decoy: None,
        shown: |_, x| format!("module `{x}`"),
    },
    Case {
        what: "a constructor a later revert removes",
        first: "add constructor T.C { z: Text }",
        created: true,
        parent: "T",
        object: "C",
        rename: None,
        rename_parent: None,
        decoy: None,
        names: "revert 0001",
        names_decoy: None,
        shown: |p, x| format!("constructor `auth::{p}.{x}`"),
    },
    Case {
        what: "a field renamed, which keeps its id",
        first: "rename field T.A.x to x2",
        created: false,
        parent: "A",
        object: "x2",
        rename: Some(("rename field T.{P}.{X} to x3", "x3")),
        rename_parent: Some(("rename constructor T.{P} to A2", "A2")),
        decoy: None,
        names: "rename field T.{P}.{X} to w",
        names_decoy: None,
        shown: |p, x| format!("field `auth::T.{p}.{x}`"),
    },
];

#[derive(Clone, Copy, Debug)]
enum Between {
    Nothing,
    Rename,
    RenameParent,
    Decoy,
}

#[derive(Clone, Copy, Debug)]
enum Later {
    Names,
    NamesDecoy,
    Neutral,
}

/// The migrations of one history after `BASE`, and those after the first that name the object,
/// with the object as named before each.
struct History {
    texts: Vec<String>,
    dependants: Vec<(u64, String)>,
}

fn spelled(t: &str, p: &str, x: &str) -> String {
    t.replace("{P}", p).replace("{X}", x)
}

fn history(c: &Case, between: Between, later: Later) -> Option<History> {
    let (mut p, mut x) = (c.parent.to_string(), c.object.to_string());
    let mut texts = vec![c.first.to_string()];
    let mut dependants = Vec::new();
    match between {
        Between::Nothing => {}
        Between::Rename => {
            let (t, to) = c.rename?;
            texts.push(spelled(t, &p, &x));
            if c.created {
                dependants.push((2, (c.shown)(&p, &x)));
            }
            x = to.to_string();
        }
        Between::RenameParent | Between::Decoy => {
            let (t, to) = c.rename_parent?;
            texts.push(spelled(t, &p, &x));
            p = to.to_string();
            if let Between::Decoy = between {
                texts.push(c.decoy?.to_string());
            }
        }
    }
    let n = texts.len() as u64 + 1;
    match later {
        Later::Names => {
            texts.push(spelled(c.names, &p, &x));
            if c.created {
                dependants.push((n, (c.shown)(&p, &x)));
            }
        }
        Later::NamesDecoy => {
            if !matches!(between, Between::Decoy) {
                return None;
            }
            texts.push(c.names_decoy?.to_string());
        }
        Later::Neutral => texts.push(NEUTRAL.to_string()),
    }
    Some(History { texts, dependants })
}

/// Returns `text` with the header of module `auth` unless it starts with a header of its own.
pub(super) fn headed(text: &str) -> String {
    if text.starts_with("module ") {
        text.to_string()
    } else {
        format!("module auth\n{text}")
    }
}

fn refused(src: &Source, text: &str) -> Vec<String> {
    match migrate(src, text) {
        Ok(s) => panic!("{text}\napplied; expected a refusal\n{}", printed_or(&s)),
        Err(errs) => errs,
    }
}

fn dependency(dependant: &(u64, String)) -> String {
    format!(
        "`revert 0001` is refused while migration {:04} depends on it: it names {}",
        dependant.0, dependant.1
    )
}

#[test]
fn a_revert_is_refused_naming_a_dependant_through_a_rename_of_the_type() {
    let s = applies(
        &running(),
        "module auth
add constructor Identity.Corp { domain: Text }
  arm display_name: Corp c -> c.domain
  arm login_present: Corp c -> false",
    );
    let s = applies(&s, "module auth\nrename type Identity to Login");
    let s = applies(&s, "module auth\nenable identities.Corp");
    assert_eq!(
        vec![
            "`revert 0001` is refused while migration 0003 depends on it: it names constructor `auth::Login.Corp`"
                .to_string()
        ],
        refused(&s, "module auth\nrevert 0001"),
    );
}

#[test]
fn a_revert_is_refused_exactly_when_a_later_migration_names_an_object_it_created() {
    let mut ran = 0;
    let mut refusals = 0;
    let mut wrong = Vec::new();
    for c in CASES {
        for between in [
            Between::Nothing,
            Between::Rename,
            Between::RenameParent,
            Between::Decoy,
        ] {
            for later in [Later::Names, Later::NamesDecoy, Later::Neutral] {
                let Some(h) = history(c, between, later) else {
                    continue;
                };
                let mut s = source(BASE);
                for t in &h.texts {
                    s = applies(&s, &headed(t));
                }
                let at = format!("{}, {between:?}, {later:?}", c.what);
                let gate = s.dependants(0);
                if gate != Ok(h.dependants.clone()) {
                    wrong.push(format!(
                        "{at}: the gate gives {gate:?}, expected {:?}",
                        h.dependants
                    ));
                }
                let got = migrate(&s, "module auth\nrevert 0001").map(|_| ());
                match h.dependants.first() {
                    Some(d) => {
                        refusals += 1;
                        if got != Err(vec![dependency(d)]) {
                            wrong.push(format!("{at}: expected {:?}, got {got:?}", dependency(d)));
                        }
                    }
                    None => {
                        if let (Between::Nothing, Later::Neutral, Err(e)) = (between, later, &got) {
                            wrong.push(format!("{at}: expected to apply, got {e:?}"));
                        }
                    }
                }
                ran += 1;
            }
        }
    }
    assert_eq!(Vec::<String>::new(), wrong);
    assert_eq!((ran, refusals), (88, 48));
}

#[test]
fn a_revert_is_refused_while_a_later_revert_undoes_what_it_created() {
    let mut s = source(BASE);
    for t in [
        "add constructor T.C { z: Text }",
        "revert 0001",
        "revert 0002",
        "enable ts.C",
    ] {
        s = applies(&s, &headed(t));
    }
    let c = "constructor `auth::T.C`".to_string();
    assert_eq!(Ok(vec![(2, c.clone()), (3, c.clone())]), s.dependants(0));
    assert_eq!(
        vec![dependency(&(2, c))],
        refused(&s, "module auth\nrevert 0001")
    );
}

#[test]
fn a_revert_is_refused_while_a_later_grant_lists_a_field_it_added() {
    let mut s = source(BASE);
    for t in [
        "add field T.A.z: Text fill \"a\"",
        "grant read ts.A (z) to support",
    ] {
        s = applies(&s, &headed(t));
    }
    assert_eq!(
        vec![dependency(&(2, "field `auth::T.A.z`".to_string()))],
        refused(&s, "module auth\nrevert 0001")
    );
}

#[test]
fn a_revert_is_refused_while_a_later_migration_declares_in_the_module_it_added() {
    let mut s = source(BASE);
    for t in ["module m\nadd module m", "module m\nadd type X = X1 | X2"] {
        s = applies(&s, t);
    }
    assert_eq!(
        vec![dependency(&(2, "module `m`".to_string()))],
        refused(&s, "module m\nrevert 0001")
    );
}

#[test]
fn a_revert_is_refused_while_a_later_migration_does_not_resolve() {
    let mut s = source(BASE);
    for t in [
        "add index (x) in ts.A",
        "adopt tzdb \"2026b\" from \"tzdata-2026b.tzif\"",
    ] {
        s = applies(&s, &headed(t));
    }
    assert_eq!(
        vec![
            "the log does not resolve: the release data `tzdata-2026b.tzif` is not available"
                .to_string()
        ],
        refused(&s, "module auth\nrevert 0001")
    );
}

/// Returns the declarations of `s` and the stage of each write state: what a revert restores. A
/// constructor a revert adds back is a new one, declared at the revert's version.
fn restored(s: &Source) -> impl PartialEq + std::fmt::Debug {
    let stages = |m: &BTreeMap<StateKey, State>| -> Vec<(StateKey, Stage)> {
        m.iter().map(|(k, v)| (k.clone(), v.stage)).collect()
    };
    (s.decls.clone(), stages(&s.states), stages(&s.value_states))
}

#[test]
fn a_revert_of_a_revert_of_a_revert_applies_as_its_inverse() {
    let mut s = vec![source(BASE)];
    for t in [
        "add constructor T.C { z: Text }",
        "revert 0001",
        "revert 0002",
        "revert 0003",
    ] {
        let next = applies(s.last().unwrap(), &headed(t));
        s.push(next);
    }
    assert_eq!(restored(&s[2]), restored(&s[4]));
    let s5 = applies(&s[4], "module auth\nrevert 0004");
    assert_eq!(restored(&s[3]), restored(&s5));
}

#[test]
fn a_revert_of_a_chain_of_reverts_restores_the_source_before_its_migration() {
    let mut ran = 0;
    for c in CASES {
        let module = if c.first.starts_with("module m") {
            "m"
        } else {
            "auth"
        };
        let mut s = vec![source(BASE)];
        for t in [
            headed(c.first),
            format!("module {module}\nrevert 0001"),
            format!("module {module}\nrevert 0002"),
            format!("module {module}\nrevert 0003"),
        ] {
            let next = applies(s.last().unwrap(), &t);
            s.push(next);
        }
        assert_eq!(restored(&s[2]), restored(&s[4]), "{}", c.what);
        ran += 1;
    }
    assert_eq!(CASES.len(), ran);
}

#[test]
fn a_qualified_grant_field_list_resolves_with_its_constructors_qualified() {
    let mut s = source(BASE);
    for t in [
        "add field T.B.x: Text fill \"b\"",
        "grant read ts.(A | B) (B.x) to support",
    ] {
        s = applies(&s, &headed(t));
    }
    let got = s
        .resolved(2)
        .map(|ops| ops.into_iter().map(|o| o.op).collect::<Vec<_>>());
    assert_eq!(
        Ok(vec![Resolved::Grant {
            privilege: "read".into(),
            target: "auth::ts . ( auth::A | auth::B ) ( auth::B . x )".into(),
            grantee: "support".into(),
            grant_option: false,
        }]),
        got
    );
}

#[test]
fn a_header_module_alone_makes_no_dependant() {
    let mut s = source(BASE);
    for t in [
        "module m\nadd module m",
        "module m\nrename field auth::U.name to label",
    ] {
        s = applies(&s, t);
    }
    assert_eq!(Ok(Vec::new()), s.dependants(0));
}

#[test]
fn reverting_drop_type_restores_the_value_states_of_its_constructors() {
    let mut s = running();
    for t in [
        "add type P = A | B",
        "add constructor P.C",
        "drop type P",
        "revert 0003",
    ] {
        s = applies(&s, &headed(t));
    }
    let stage = |c: &str| {
        s.value_states
            .get(&("auth".to_string(), "P".to_string(), c.to_string()))
            .map(|st| st.stage)
    };
    assert_eq!(
        [
            Some(Stage::Enabled),
            Some(Stage::Enabled),
            Some(Stage::Declared)
        ],
        [stage("A"), stage("B"), stage("C")]
    );
}

/// Returns each named set of relation `identities` by name, with its id.
fn identity_sets(s: &Source) -> BTreeMap<String, u64> {
    let schema = ids_of(s);
    let r = rel(&schema, "identities");
    schema
        .ids()
        .set_ids()
        .filter(|(of, ..)| *of == r)
        .map(|(_, name, id)| (name.to_string(), id.raw()))
        .collect()
}

#[test]
fn reverting_a_rename_renames_the_object_it_renamed_after_its_name_is_reused() {
    let start = running();
    let oauth = identity_sets(&start)["OAuth"];
    let mut s = start;
    for t in [
        "rename set OAuth of identities to Federated",
        "rename set Federated of identities to Social",
        "add set Federated of identities = Google",
    ] {
        s = applies(&s, &headed(t));
    }
    let federated = identity_sets(&s)["Federated"];
    let s = applies(&s, "module auth\nrevert 0001");
    assert_eq!(
        BTreeMap::from([
            ("Federated".to_string(), federated),
            ("OAuth".to_string(), oauth)
        ]),
        identity_sets(&s)
    );
    assert_eq!(
        "set OAuth of identities = Google | Apple",
        decl(&s, "set", "identities.OAuth")
    );
    assert_eq!(
        "set Federated of identities = Google",
        decl(&s, "set", "identities.Federated")
    );
}
