use super::*;

fn run(text: &str) -> Result<Adhoc, Vec<String>> {
    let file = parse_schema(
        "module app
         type Row = { id: I64, public: Text, secret: Text }
         relation left : Row key id
         relation right : Row key id
         fn left_rows () -> Set Row = left
         fn fixed_rows (x: Row) -> Set Row = left
         fn rebuild (x: Row) -> Row = Row { x with public: \"copy\" }",
        Options::default(),
    )
    .ok()
    .expect("the provenance schema parses");
    let s = build(&[(FileId(0), file)]).expect("the provenance schema builds");
    let program = check_schema(&s).expect("the provenance schema checks");
    let items = parse_adhoc(text, Options::default())
        .ok()
        .expect("the provenance statement parses");
    check_adhoc(&s, &program, &items, &BTreeMap::new())
        .map_err(|ds| ds.into_iter().map(|d| d.message).collect())
}

#[test]
fn fix_identity_update_preserves_snapshot_relation() {
    let text = "update x in (fix rows = left union rows in rows) set x";
    accept(text);
}

fn accept(text: &str) {
    if let Err(errors) = run(text) {
        panic!("{text}: {errors:?}");
    }
}

fn refuse(text: &str, needle: &str) {
    let Err(errors) = run(text) else {
        panic!("{text}: accepted");
    };
    assert!(
        errors.iter().any(|e| e.contains(needle)),
        "{text}: {errors:?}"
    );
}

#[test]
fn fix_preserves_filtered_aliases_and_mutual_origins() {
    for query in [
        "fix rows = left union rows in rows",
        "fix rows = left union (from x in rows where x.id > 0 select x) in rows",
        "fix a = b, b = c, c = left union (from x in a select x) in a",
        "fix rows = left_rows() union rows in rows",
        "fix rows = left union (from x in rows, y in fixed_rows(x) select y) in rows",
        "fix rows = left union rows.Row in rows.Row",
        "fix rows = left union rows in from x in rows where key x == left.key(x.id) select x",
        "fix rows = left union rows in (fix alias = rows union alias in alias)",
    ] {
        accept(&format!(
            "update x in ({query}) set Row {{ x with public: \"changed\" }}"
        ));
        accept(&format!("delete x in ({query})"));
    }
}

#[test]
fn fix_preserves_empty_recursive_arms_and_body_unions() {
    for query in [
        "fix rows = left union rows union [] in rows",
        "fix rows = [] union left union rows in rows",
        "fix rows = left union rows in (rows union [])",
        "fix rows = left union rows in ([] union rows)",
        "fix rows = left union rows in (([] union []) union rows)",
        "fix rows = left union rows in (rows union ([] union []))",
        "fix rows = left union (if True then rows else []) in rows",
        "fix rows = left union (fix empty = from x in empty, y in left select y in empty) in rows",
    ] {
        accept(&format!("update x in ({query}) set x"));
        accept(&format!("delete x in ({query})"));
    }
}

#[test]
fn fix_rejects_mixed_constructed_and_unanchored_rows() {
    for query in [
        "fix rows = left union right union rows in rows",
        "fix a = b, b = left union right union a in a",
        "fix rows = left union [Row { id: 1, public: \"a\", secret: \"b\" }] union rows in rows",
        "fix rows = left union (from x in rows select Row { x with public: \"copy\" }) in rows",
        "fix rows = left union (from x in rows select rebuild(x)) in rows",
        "fix rows = left union rows in from x in rows select Row { x with public: \"copy\" }",
        "fix rows = from x in rows, seed in left select seed in rows",
        "fix rows = left union (fix inner = right union rows in inner) in rows",
    ] {
        refuse(
            &format!("update x in ({query}) set x"),
            "ranges over rows of one relation variable",
        );
        refuse(
            &format!("delete x in ({query})"),
            "ranges over rows of one relation variable",
        );
    }
}

#[test]
fn fix_rejects_rows_proposed_by_writes() {
    for write in [
        "insert left from left",
        "update x in left set x",
        "delete x in left",
    ] {
        for statement in ["update x in rows set x", "delete x in rows"] {
            refuse(
                &format!(
                    "transact let staged = {write} let rows = fix rows = staged union rows in rows {statement}"
                ),
                "a write proposes",
            );
        }
    }
}

#[test]
fn empty_union_context_does_not_guess_types_or_allow_incompatible_operands() {
    refuse("[] union []", "the type of `[]` is not known");
    refuse("left union [1]", "no common type");
    refuse("[1] union left", "no common type");
    accept("(from x in left group x by x.id into g select (g union []))");
}

#[test]
fn expression_aliases_preserve_write_proposal_taint() {
    for statement in ["update x in rows set x", "delete x in rows"] {
        refuse(
            &format!(
                "transact let staged = update x in left set x let alias = staged let rows = alias {statement}"
            ),
            "a write proposes",
        );
        refuse(
            &format!(
                "transact let staged = insert left Row {{ id: 1, public: \"a\", secret: \"b\" }} let rows = fix rows = [staged] union rows in rows {statement}"
            ),
            "a write proposes",
        );
    }
}

#[test]
fn direct_proposed_alias_update_is_refused() {
    refuse(
        "transact let staged = update x in left set x let rows = staged update x in rows set x",
        "a write proposes",
    );
}

#[test]
fn direct_proposed_alias_delete_is_refused() {
    refuse(
        "transact let staged = update x in left set x let rows = staged delete x in rows",
        "a write proposes",
    );
}

#[test]
fn snapshot_aliases_remain_writable() {
    for statement in ["update x in rows set x", "delete x in rows"] {
        accept(&format!(
            "transact let source = left let alias = source let rows = fix rows = alias union rows in rows {statement}"
        ));
    }
    accept(
        "transact let row = match left.key(1) | Row x -> x let alias = row update alias set alias",
    );
    accept("transact let row = match left.key(1) | Row x -> x let alias = row delete alias");
    accept("match left.key(1) | Row x -> update x set x");
    refuse(
        "transact let row = insert left Row { id: 1, public: \"a\", secret: \"b\" } match row | Row x -> delete x",
        "is not a row of the snapshot",
    );
}

#[test]
fn upsert_snapshot_and_proposed_binders_keep_their_write_rules() {
    accept(
        "insert left Row { id: 1, public: \"a\", secret: \"b\" } unless conflict on key else update old set old",
    );
    accept(
        "insert left Row { id: 1, public: \"a\", secret: \"b\" } unless conflict on key else delete old",
    );
    refuse(
        "insert left Row { id: 1, public: \"a\", secret: \"b\" } unless conflict on key else update new set new",
        "is not a row of the snapshot",
    );
    refuse(
        "insert left Row { id: 1, public: \"a\", secret: \"b\" } unless conflict on key else delete new",
        "is not a row of the snapshot",
    );
}

/// Returns the headings of the unused-deref conformance cases whose key or row has `provenance`.
fn unused_deref_cases(provenance: Provenance) -> &'static [&'static str] {
    match provenance {
        Provenance::Snapshot => &[
            "A row read through a reference, from a function or by a match over a stored row is a stored row.",
        ],
        Provenance::Reference => &[
            "A reference field of a stored row names a row.",
            "A reference read from a stored row and bound by a generator names a row.",
        ],
        Provenance::Proposed => &[
            "A reference field of the row an upsert proposes, which no constraint checks when it conflicts.",
            "A generator binder over rows a write proposes, whose reference names a row the same block inserts.",
            "A match binder over a row a write proposes.",
            "A computed field of a row a write proposes.",
            "A group binder over rows a write proposes.",
            "A generator binder over a set literal of a row a write proposes.",
            "A generator binder over a union with rows a write proposes.",
            "A generator binder over a fixpoint of rows a write proposes.",
            "An aggregate binder over rows a write proposes.",
            "A generator binder over a let bound to rows a write proposes.",
            "A match binder over the one row a write proposes.",
            "A computed field of the one row a write proposes.",
            "A match binder of the row an upsert proposes.",
        ],
        Provenance::Built => &[
            "A key built by r.key.",
            "A reference field of a constructor literal.",
            "A reference field of a one-constructor literal.",
            "A field of an anonymous record.",
            "A reference field of a stored row's copy made with `with`.",
            "A field of an anonymous record's copy made with `with`.",
            "A comprehension binder over a set of keys.",
            "A function's key parameter.",
            "A reference field of a function's row parameter.",
            "A function's key result.",
            "A reference field of a function's built row.",
            "The result of an `if`.",
            "The result of a `match`.",
            "A reference field of a binder a match on a constructor literal binds.",
            "A key an optional value holds.",
            "The key of a stored row.",
            "A group's key.",
        ],
    }
}

#[test]
fn every_provenance_has_unused_deref_cases_and_every_case_a_provenance() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/queries/15-unused-deref.test");
    let text = std::fs::read_to_string(&path).expect("the unused-deref conformance file");
    let headings: Vec<&str> = text
        .lines()
        .skip(1)
        .filter_map(|l| l.strip_prefix("# "))
        .collect();
    let mut listed = Vec::new();
    for provenance in [
        Provenance::Snapshot,
        Provenance::Reference,
        Provenance::Proposed,
        Provenance::Built,
    ] {
        let cases = unused_deref_cases(provenance);
        assert!(!cases.is_empty(), "{provenance:?} has no case");
        for case in cases {
            assert!(headings.contains(case), "{provenance:?}: no case `{case}`");
        }
        listed.extend_from_slice(cases);
    }
    for heading in &headings {
        assert!(
            listed.contains(heading),
            "case `{heading}` has no provenance"
        );
    }
    assert_eq!(headings.len(), listed.len());
}
