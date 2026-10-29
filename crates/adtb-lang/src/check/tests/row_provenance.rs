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
