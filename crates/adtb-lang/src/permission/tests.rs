use super::*;
use crate::ast::{BundleItem, ExampleItem};
use crate::check::{Checker, Context, check_adhoc, check_query, check_schema, check_txn};
use crate::parse::{Options, parse_adhoc, parse_example, parse_schema, parse_type};
use crate::resolve::Visible;
use crate::span::FileId;

const SCHEMA: &str = r"
module app
type Row = { id: I64, public: Text, secret: Text }
relation left : Row key id
relation right : Row key id
type Flow { id: I64, secret: Text }
  = Draft { public: Text } | Sent { public: Text } | Void
relation flow : Flow key id
type Link = { id: I64, target: Key left }
relation links : Link key id
fn public_of (r: Row) -> Text = r.public
fn indirect (r: Row) -> Text = public_of(r)
fn apply (f: Row -> Text, r: Row) -> Text = f(r)
fn ignore (r: Row) -> I64 = 0
fn all () -> Set Row = left
relation rejects : RejectedRow key (load, line)
";

fn schema(extra: &str) -> Schema {
    let text = format!("{SCHEMA}\n{extra}");
    let file = parse_schema(&text, Options::default())
        .ok()
        .expect("schema parses");
    crate::schema::build(&[(FileId(0), file)]).expect("schema builds")
}

fn requirements(s: &Schema, params: &[(&str, &str)], text: &str) -> Requirements {
    let program = check_schema(s).expect("schema checks");
    let checker = Checker::new(s, Visible::all(s), Context::Adhoc);
    let params = params
        .iter()
        .map(|(name, ty)| {
            let ty = parse_type(ty, Options::default())
                .ok()
                .expect("type parses");
            (
                name.to_string(),
                checker.resolve_type(&ty).expect("type checks"),
            )
        })
        .collect();
    let items = parse_adhoc(text, Options::default())
        .ok()
        .expect("text parses");
    let checked =
        check_adhoc(s, &program, &items, &params).unwrap_or_else(|e| panic!("{text}: {e:?}"));
    adhoc(s, &program, &checked)
        .expect("requirements analyze")
        .remove(0)
}

fn target(s: &Schema, name: &str, ctor: &str) -> Target {
    let r = RelId(
        s.relations
            .iter()
            .position(|r| r.name == name)
            .expect("relation") as u32,
    );
    let d = s.relation(r).elem_def;
    let c = s.ty(d).ctor_index(ctor).expect("constructor");
    Target {
        relation: RelationId(s.ids.relations[&r]),
        constructor: ConstructorId(s.ids.ctors[&(d, c)]),
    }
}

fn field(s: &Schema, relation: &str, ctor: &str, name: &str) -> FieldId {
    let r = s
        .relations
        .iter()
        .find(|r| r.name == relation)
        .expect("relation");
    let d = r.elem_def;
    let c = s.ty(d).ctor_index(ctor).expect("constructor");
    let own = (!s.ty(d).header().iter().any(|f| f.name == name)).then_some(c);
    FieldId(s.ids.fields[&(d, own, name.to_string())])
}

fn reads(s: &Schema, relation: &str, ctor: &str, names: &[&str]) -> BTreeSet<Requirement> {
    let t = target(s, relation, ctor);
    std::iter::once(Requirement::ReadConstructor(t))
        .chain(
            std::iter::once("id")
                .chain(names.iter().copied())
                .map(|f| Requirement::ReadField(t, field(s, relation, ctor, f))),
        )
        .collect()
}

fn assert_exact(got: &Requirements, want: BTreeSet<Requirement>) {
    assert_eq!(got.iter().cloned().collect::<BTreeSet<_>>(), want);
}

#[test]
fn projection_distinguishes_relations_with_one_row_type() {
    let s = schema("");
    let got = requirements(&s, &[], "from x in left select x.public");
    assert_exact(&got, reads(&s, "left", "Row", &["public"]));
    let got = requirements(&s, &[], "from x in right select x.secret");
    assert_exact(&got, reads(&s, "right", "Row", &["secret"]));
}

#[test]
fn parameters_do_not_fabricate_relation_origins() {
    let s = schema("");
    let got = requirements(&s, &[("x", "Row")], "x.secret");
    assert_exact(&got, BTreeSet::new());
}

#[test]
fn range_count_and_exists_read_only_implicit_keys() {
    let s = schema("");
    for text in [
        "from x in left select key x",
        "count(x for x in left)",
        "exists x in left where true",
    ] {
        assert_exact(&requirements(&s, &[], text), reads(&s, "left", "Row", &[]));
    }
}

#[test]
fn whole_and_nested_rows_read_all_fields() {
    let s = schema("");
    for text in ["left", "from x in left select { nested: x }"] {
        assert_exact(
            &requirements(&s, &[], text),
            reads(&s, "left", "Row", &["public", "secret"]),
        );
    }
}

#[test]
fn constructor_arms_read_only_their_fields() {
    let s = schema("");
    let text =
        "from x in flow.(Draft | Sent) select match x | Draft d -> d.public | Sent s -> s.secret";
    let want = reads(&s, "flow", "Draft", &["public"])
        .union(&reads(&s, "flow", "Sent", &["secret"]))
        .cloned()
        .collect();
    assert_exact(&requirements(&s, &[], text), want);
}

#[test]
fn functions_higher_order_and_aliases_preserve_origin() {
    let s = schema("");
    for text in [
        "from x in left select indirect(x)",
        "from x in left select apply(public_of, x)",
        "transact let rows = all() return from y in rows select y.public",
    ] {
        assert_exact(
            &requirements(&s, &[], text),
            reads(&s, "left", "Row", &["public"]),
        );
    }
}

#[test]
fn unchanged_secret_carries_without_read_or_update() {
    let s = schema("");
    let mut want = reads(&s, "left", "Row", &[]);
    want.insert(Requirement::UpdateField(
        target(&s, "left", "Row"),
        field(&s, "left", "Row", "public"),
    ));
    let text = "update x in left set Row { x with public: \"changed\" }";
    assert_exact(&requirements(&s, &[], text), want);
}

#[test]
fn explicit_secret_predicate_adds_a_read() {
    let s = schema("");
    let mut want = reads(&s, "left", "Row", &["secret"]);
    want.insert(Requirement::UpdateField(
        target(&s, "left", "Row"),
        field(&s, "left", "Row", "public"),
    ));
    let text = "update x in left where x.secret == \"s\" set Row { x with public: \"changed\" }";
    assert_exact(&requirements(&s, &[], text), want);
}

#[test]
fn identity_arms_require_only_true_retag_edges() {
    let s = schema("");
    let mut want = reads(&s, "flow", "Draft", &[]);
    want.extend(reads(&s, "flow", "Sent", &[]));
    want.extend(reads(&s, "flow", "Void", &[]));
    want.insert(Requirement::Retag {
        relation: target(&s, "flow", "Draft").relation,
        from: target(&s, "flow", "Draft").constructor,
        to: target(&s, "flow", "Sent").constructor,
    });
    let text = "update x in flow set match x | Draft d -> Sent { public: \"sent\" } | Sent s -> s | Void v -> v";
    assert_exact(&requirements(&s, &[], text), want);
}

#[test]
fn delete_and_insert_remain_separate_rights() {
    let s = schema("");
    let mut want = reads(&s, "left", "Row", &[]);
    want.insert(Requirement::Delete(target(&s, "left", "Row")));
    assert_exact(&requirements(&s, &[], "delete x in left"), want);
    let got = requirements(
        &s,
        &[],
        "insert left { id: 1, public: \"p\", secret: \"s\" }",
    );
    assert_exact(
        &got,
        BTreeSet::from([Requirement::Insert(target(&s, "left", "Row"))]),
    );
}

#[test]
fn projected_carry_is_an_explicit_read_without_tainting_shared_carries() {
    let s = schema("");
    let mut want = reads(&s, "left", "Row", &["secret"]);
    want.insert(Requirement::UpdateField(
        target(&s, "left", "Row"),
        field(&s, "left", "Row", "public"),
    ));
    let text = r#"update x in left set Row { x with public: (Row { x with public: "p" }).secret }"#;
    assert_exact(&requirements(&s, &[], text), want);
    let text = r#"update x in left set Row { x with public: "p" }"#;
    assert!(!requirements(&s, &[], text).iter().any(|r| *r
        == Requirement::ReadField(
            target(&s, "left", "Row"),
            field(&s, "left", "Row", "secret")
        )));
}

#[test]
fn reconstructed_query_results_consume_carried_fields() {
    let s = schema("");
    let text = r#"from x in left select Row { x with public: "changed" }"#;
    assert_exact(
        &requirements(&s, &[], text),
        reads(&s, "left", "Row", &["secret"]),
    );
}

#[test]
fn old_aliases_prove_identity_but_fresh_function_reads_do_not() {
    let s =
        schema("fn copy (r: Row) -> Row = r\nfn fetch (k: Key left) -> Row = match k | Row r -> r");
    for text in [
        "update x in left set copy(x)",
        "update x in left set copy(copy(x))",
    ] {
        assert_exact(&requirements(&s, &[], text), reads(&s, "left", "Row", &[]));
    }
    let text = "update x in all() set fetch(key x)";
    let got = requirements(&s, &[], text);
    let mut want = reads(&s, "left", "Row", &["public", "secret"]);
    for name in ["public", "secret"] {
        want.insert(Requirement::UpdateField(
            target(&s, "left", "Row"),
            field(&s, "left", "Row", name),
        ));
    }
    assert_exact(&got, want);
}

#[test]
fn following_references_reads_both_actual_relations() {
    let s = schema("");
    let text = "from x in links select match x.target | Row r -> r.public";
    let mut want = reads(&s, "links", "Link", &["target"]);
    want.extend(reads(&s, "left", "Row", &["public"]));
    assert_exact(&requirements(&s, &[], text), want);
}

#[test]
fn comparison_grouping_and_ordering_consume_actual_fields() {
    let s = schema("");
    for text in [
        r#"from x in left where x.secret == "s" select x.public"#,
        "from x in left group x by x.secret into g select { secret: g.key, n: count(y for y in g) }",
        "from x in left select { public: x.public, secret: x.secret, id: x.id } order by (x.secret, x.id)",
    ] {
        let names = if text.contains("public") {
            vec!["public", "secret"]
        } else {
            vec!["secret"]
        };
        assert_exact(
            &requirements(&s, &[], text),
            reads(&s, "left", "Row", &names),
        );
    }
}

#[test]
fn fix_propagates_origins_until_stable() {
    let s = schema("");
    let text = "fix a = left union b, b = from x in a select x in from x in b select x.secret";
    assert_exact(
        &requirements(&s, &[], text),
        reads(&s, "left", "Row", &["secret"]),
    );
}

#[test]
fn user_aggregate_reaches_parts_without_reading_the_whole_input_row() {
    let s = schema(
        r#"
aggregate secrets (x: Row) -> I64 =
  state { n: I64 } init { n: 0 }
  step (st,x) -> { n: st.n + (if x.secret == "s" then 1 else 0) }
  merge (a,b) -> { n: a.n + b.n }
  finish st -> st.n
"#,
    );
    let text = "secrets(x for x in left)";
    assert_exact(
        &requirements(&s, &[], text),
        reads(&s, "left", "Row", &["secret"]),
    );
}

#[test]
fn file_capability_is_separate_from_relation_grant_option() {
    let s = schema("");
    let got = requirements(
        &s,
        &[],
        r#"from x in read_csv("input.csv", Row) select x.public"#,
    );
    assert_exact(&got, BTreeSet::from([Requirement::ReadFiles]));
    let mut grants = EffectiveGrants::default();
    assert_eq!(
        got.missing(&grants, Mode::Registration),
        vec![Requirement::ReadFiles]
    );
    grants.read_files = true;
    assert!(got.missing(&grants, Mode::Registration).is_empty());
}

#[test]
fn upsert_implicit_lookup_needs_insert_only() {
    let s = schema("");
    let text =
        r#"insert left { id: 1, public: "p", secret: "s" } unless conflict on key else skip"#;
    assert_exact(
        &requirements(&s, &[], text),
        BTreeSet::from([Requirement::Insert(target(&s, "left", "Row"))]),
    );
}

#[test]
fn upsert_old_reads_and_new_source_origins_stay_distinct() {
    let s = schema("");
    let text = r#"insert left { id: 1, public: "p", secret: "s" } unless conflict on key else update old set Row { old with public: old.secret }"#;
    let mut want = reads(&s, "left", "Row", &["secret"]);
    want.insert(Requirement::Insert(target(&s, "left", "Row")));
    want.insert(Requirement::UpdateField(
        target(&s, "left", "Row"),
        field(&s, "left", "Row", "public"),
    ));
    assert_exact(&requirements(&s, &[], text), want);
    let text = "insert left from (from x in right select x) unless conflict on key else update old set Row { old with public: new.public }";
    let mut want = reads(&s, "right", "Row", &["public", "secret"]);
    want.insert(Requirement::Insert(target(&s, "left", "Row")));
    want.insert(Requirement::UpdateField(
        target(&s, "left", "Row"),
        field(&s, "left", "Row", "public"),
    ));
    assert_exact(&requirements(&s, &[], text), want);
}

#[test]
fn bound_upsert_reads_conflicting_old_provenance() {
    let s = schema("");
    let text = r#"transact let x = insert left { id: 1, public: "p", secret: "s" } unless conflict on key else skip return x.secret"#;
    let mut want = reads(&s, "left", "Row", &["secret"]);
    want.insert(Requirement::Insert(target(&s, "left", "Row")));
    assert_exact(&requirements(&s, &[], text), want);
}

#[test]
fn bound_update_preserves_carried_field_origins() {
    let s = schema("");
    let text = r#"transact let rows = update x in left set Row { x with public: "p" } return from x in rows select x.secret"#;
    let mut want = reads(&s, "left", "Row", &["secret"]);
    want.insert(Requirement::UpdateField(
        target(&s, "left", "Row"),
        field(&s, "left", "Row", "public"),
    ));
    assert_exact(&requirements(&s, &[], text), want);
}

#[test]
fn rejects_requires_its_destination_insert() {
    let s = schema("");
    let text =
        r#"insert left from (from x in read_csv("input.csv", Row) select x) rejects into rejects"#;
    let mut want = BTreeSet::from([
        Requirement::ReadFiles,
        Requirement::Insert(target(&s, "left", "Row")),
    ]);
    want.insert(Requirement::Insert(target(&s, "rejects", "RejectedRow")));
    assert_exact(&requirements(&s, &[], text), want);
}

#[test]
fn coverage_uses_explicit_snapshots_and_relation_grant_option() {
    let s = schema("");
    let got = requirements(&s, &[], "from x in left select x.secret");
    let grant = EffectiveGrant {
        right: Right::Read {
            target: target(&s, "left", "Row"),
            fields: BTreeSet::from([field(&s, "left", "Row", "secret")]),
        },
        grant_option: false,
    };
    let mut grants = EffectiveGrants {
        relations: vec![grant],
        read_files: false,
    };
    assert!(got.missing(&grants, Mode::Ordinary).is_empty());
    assert_eq!(
        got.missing(&grants, Mode::Registration),
        got.iter().cloned().collect::<Vec<_>>()
    );
    grants.relations[0].grant_option = true;
    assert!(got.missing(&grants, Mode::Registration).is_empty());
    grants.relations.clear();
    let missing = got.missing(&grants, Mode::Ordinary);
    assert_eq!(missing, got.iter().cloned().collect::<Vec<_>>());
    assert!(missing.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn key_reads_are_implicit_in_even_an_empty_read_field_grant() {
    let s = schema("");
    let got = requirements(&s, &[], "count(x for x in left)");
    let grants = EffectiveGrants {
        relations: vec![EffectiveGrant {
            right: Right::Read {
                target: target(&s, "left", "Row"),
                fields: BTreeSet::new(),
            },
            grant_option: true,
        }],
        read_files: false,
    };
    assert!(got.missing(&grants, Mode::Registration).is_empty());
}

#[test]
fn keyless_relations_require_no_physical_identity_field() {
    let text = "module app\ntype Plain = { public: Text }\nrelation plain : Plain";
    let parsed = parse_schema(text, Options::default()).ok().expect("parses");
    let errors = crate::schema::build(&[(FileId(0), parsed)])
        .expect_err("keyless relations are not checked input");
    assert!(
        errors
            .iter()
            .any(|e| e.diag.message == "a relation declares exactly one key")
    );
}

#[test]
fn normalization_discards_unused_total_arguments_without_authority() {
    let s = schema("fn ignore_set (xs: Set Row) -> I64 = 0");
    assert_exact(&requirements(&s, &[], "ignore_set(left)"), BTreeSet::new());
    assert_exact(
        &requirements(&s, &[], "from x in left select ignore(x)"),
        reads(&s, "left", "Row", &[]),
    );
}

#[test]
fn renames_preserve_stable_requirement_identities() {
    use crate::evolve::{Source, SourceDecl};
    let source = Source::new(vec![
        SourceDecl {
            module: "app".to_string(),
            text: "type Item = Entry { id: I64, secret: Text }".to_string(),
        },
        SourceDecl {
            module: "app".to_string(),
            text: "relation items : Item key id".to_string(),
        },
    ])
    .expect("source builds");
    let before = requirements(
        &source.schema().expect("schema"),
        &[],
        "from x in items select x.secret",
    );
    let text = "module app\nrename field Item.Entry.secret to hidden\nrename constructor Item.Entry to Renamed\nrename relation items to renamed\nrename type Item to Thing";
    let migration = crate::parse::parse_migration(text, Options::default())
        .ok()
        .expect("migration parses");
    let source = source.migrate(&migration, text).expect("renames apply");
    let after = requirements(
        &source.schema().expect("schema"),
        &[],
        "from x in renamed select x.hidden",
    );
    assert_eq!(before, after);
}

#[test]
fn a_reachable_missing_catalog_identity_is_an_explicit_error() {
    let mut s = schema("");
    let rel = RelId(
        s.relations
            .iter()
            .position(|r| r.name == "left")
            .expect("left") as u32,
    );
    s.ids.relations.remove(&rel);
    let program = check_schema(&s).expect("schema checks");
    let items = parse_adhoc("count(x for x in left)", Options::default())
        .ok()
        .expect("parses");
    let checked = check_adhoc(&s, &program, &items, &BTreeMap::new()).expect("checks");
    assert_eq!(
        adhoc(&s, &program, &checked),
        Err(Error::MissingIdentity(crate::ids::Slot::Relation(rel)))
    );
}

#[test]
fn fix_identity_updates_keep_their_actual_old_copy_evidence() {
    let s = schema("");
    let text = "update x in (fix rows = left union rows in rows) set x";
    assert_exact(&requirements(&s, &[], text), reads(&s, "left", "Row", &[]));
}

#[test]
fn returning_a_whole_group_consumes_its_rows() {
    let s = schema("");
    let text = "from x in left group x by x.public into g select g";
    assert_exact(
        &requirements(&s, &[], text),
        reads(&s, "left", "Row", &["public", "secret"]),
    );
}

#[test]
fn comparing_a_whole_row_reads_its_actual_origin_only() {
    let s = schema("");
    let text = "from x in left select x == parameter";
    assert_exact(
        &requirements(&s, &[("parameter", "Row")], text),
        reads(&s, "left", "Row", &["public", "secret"]),
    );
}

#[test]
fn retag_edges_do_not_form_a_source_destination_cross_product() {
    let s = schema("");
    let text = r#"update x in flow set match x | Draft d -> Sent { public: "s" } | Sent s -> Void {} | Void v -> v"#;
    let mut want = reads(&s, "flow", "Draft", &[]);
    want.extend(reads(&s, "flow", "Sent", &[]));
    want.extend(reads(&s, "flow", "Void", &[]));
    for (from, to) in [("Draft", "Sent"), ("Sent", "Void")] {
        want.insert(Requirement::Retag {
            relation: target(&s, "flow", from).relation,
            from: target(&s, "flow", from).constructor,
            to: target(&s, "flow", to).constructor,
        });
    }
    assert_exact(&requirements(&s, &[], text), want);
}

#[test]
fn implicit_cascade_actions_add_no_user_rights() {
    let s = schema(
        "type Child = { id: I64, parent: Key left on delete cascade }\nrelation children : Child key id",
    );
    let mut want = reads(&s, "left", "Row", &[]);
    want.insert(Requirement::Delete(target(&s, "left", "Row")));
    assert_exact(&requirements(&s, &[], "delete x in left"), want);
}

#[test]
fn coverage_separates_updates_inserts_deletes_and_retags() {
    let s = schema("");
    let t = target(&s, "left", "Row");
    let read = EffectiveGrant {
        right: Right::Read {
            target: t,
            fields: BTreeSet::new(),
        },
        grant_option: true,
    };
    let mut grants = EffectiveGrants {
        relations: vec![read],
        read_files: false,
    };
    let updated = requirements(
        &s,
        &[],
        r#"update x in left set Row { x with public: "p" }"#,
    );
    let field = field(&s, "left", "Row", "public");
    assert_eq!(
        updated.missing(&grants, Mode::Ordinary),
        vec![Requirement::UpdateField(t, field)]
    );
    grants.relations.push(EffectiveGrant {
        right: Right::Update {
            target: t,
            fields: BTreeSet::from([field]),
        },
        grant_option: false,
    });
    assert!(updated.missing(&grants, Mode::Ordinary).is_empty());
    assert_eq!(
        updated.missing(&grants, Mode::Registration),
        vec![Requirement::UpdateField(t, field)]
    );
    let deleted = requirements(&s, &[], "delete x in left");
    assert_eq!(
        deleted.missing(&grants, Mode::Ordinary),
        vec![Requirement::Delete(t)]
    );
    grants.relations.push(EffectiveGrant {
        right: Right::Delete(t),
        grant_option: true,
    });
    assert!(deleted.missing(&grants, Mode::Registration).is_empty());
    let inserted = requirements(&s, &[], r#"insert left {id: 1, public: "p", secret: "s"}"#);
    assert_eq!(
        inserted.missing(&grants, Mode::Ordinary),
        vec![Requirement::Insert(t)]
    );
    grants.relations.push(EffectiveGrant {
        right: Right::Insert(t),
        grant_option: true,
    });
    assert!(inserted.missing(&grants, Mode::Registration).is_empty());
}

#[test]
fn coverage_requires_retag_even_with_insert_and_delete() {
    let s = schema("");
    let draft = target(&s, "flow", "Draft");
    let sent = target(&s, "flow", "Sent");
    let got = requirements(
        &s,
        &[],
        r#"update x in flow.Draft set Sent { public: "s" }"#,
    );
    let mut grants = EffectiveGrants {
        relations: vec![
            EffectiveGrant {
                right: Right::Read {
                    target: draft,
                    fields: BTreeSet::new(),
                },
                grant_option: true,
            },
            EffectiveGrant {
                right: Right::Delete(draft),
                grant_option: true,
            },
            EffectiveGrant {
                right: Right::Insert(sent),
                grant_option: true,
            },
        ],
        read_files: false,
    };
    let missing = Requirement::Retag {
        relation: draft.relation,
        from: draft.constructor,
        to: sent.constructor,
    };
    assert_eq!(got.missing(&grants, Mode::Ordinary), vec![missing]);
    grants.relations.push(EffectiveGrant {
        right: Right::Retag {
            relation: draft.relation,
            from: draft.constructor,
            to: sent.constructor,
        },
        grant_option: true,
    });
    assert!(got.missing(&grants, Mode::Registration).is_empty());
}

#[test]
fn partial_constructor_grants_report_every_missing_constructor_field() {
    let s = schema("");
    let got = requirements(&s, &[], "from x in flow.(Draft | Sent) select x.public");
    let grants = EffectiveGrants {
        relations: vec![EffectiveGrant {
            right: Right::Read {
                target: target(&s, "flow", "Draft"),
                fields: BTreeSet::from([field(&s, "flow", "Draft", "public")]),
            },
            grant_option: true,
        }],
        read_files: false,
    };
    assert_eq!(
        got.missing(&grants, Mode::Registration),
        reads(&s, "flow", "Sent", &["public"])
            .into_iter()
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_failing_parallel_upsert_branch_binds_no_old_row() {
    let s = schema("");
    let insert = Requirement::Insert(target(&s, "left", "Row"));
    let text = r#"transact let x = insert left { id: 1, public: "p", secret: "s" } unless conflict on key else (skip, fail "taken") return x.secret"#;
    assert_exact(
        &requirements(&s, &[], text),
        BTreeSet::from([insert.clone()]),
    );
    let text = r#"transact let x = insert left { id: 1, public: "p", secret: "s" } unless conflict on key else (skip, skip) return x.secret"#;
    let mut want = reads(&s, "left", "Row", &["secret"]);
    want.insert(insert);
    assert_exact(&requirements(&s, &[], text), want);
}

fn registered(s: &Schema, text: &str) -> Requirements {
    let program = check_schema(s).expect("schema checks");
    let module = s.module_named("app").expect("module app");
    let items = parse_example(text, Options::default())
        .ok()
        .expect("declaration parses");
    match items.into_iter().next() {
        Some(ExampleItem::BundleItem(BundleItem::Query(q))) => {
            let q =
                check_query(s, &program, module, &q).unwrap_or_else(|e| panic!("{text}: {e:?}"));
            query(s, &program, &q).expect("requirements analyze")
        }
        Some(ExampleItem::BundleItem(BundleItem::Txn(t))) => {
            let t = check_txn(s, &program, module, &t).unwrap_or_else(|e| panic!("{text}: {e:?}"));
            txn(s, &program, &t).expect("requirements analyze")
        }
        _ => panic!("{text}\nis not a query or transaction"),
    }
}

#[test]
fn registered_queries_read_only_their_relation_origins() {
    let s = schema("");
    let text =
        "query q (r: Row) -> Set Text = from x in left where x.public == r.public select x.secret";
    assert_exact(
        &registered(&s, text),
        reads(&s, "left", "Row", &["public", "secret"]),
    );
    let text = "query q (r: Row) -> Set Text = from x in right where x.id == r.id select r.secret";
    assert_exact(&registered(&s, text), reads(&s, "right", "Row", &[]));
}

#[test]
fn registered_transactions_carry_without_reading() {
    let s = schema("");
    let mut want = reads(&s, "left", "Row", &[]);
    want.insert(Requirement::UpdateField(
        target(&s, "left", "Row"),
        field(&s, "left", "Row", "public"),
    ));
    let text = r#"txn t (k: I64) -> I64 =
  transact
    update x in left where x.id == k set Row { x with public: "p" }
    return k"#;
    assert_exact(&registered(&s, text), want.clone());
    let text = r#"txn t (k: I64) -> Set Text =
  transact
    let rows = update x in left where x.id == k set Row { x with public: "p" }
    return from x in rows select x.secret"#;
    want.extend(reads(&s, "left", "Row", &["secret"]));
    assert_exact(&registered(&s, text), want);
}

#[test]
fn user_aggregates_charge_external_reads_in_every_part() {
    let probe = r#"(if exists y in right where y.secret == "s" then 1 else 0)"#;
    let parts = [
        (
            "init",
            format!("init {{ n: {probe} }} step (st,x) -> st merge (a,b) -> a finish st -> st.n"),
        ),
        (
            "step",
            format!(
                "init {{ n: 0 }} step (st,x) -> {{ n: st.n + {probe} }} merge (a,b) -> a finish st -> st.n"
            ),
        ),
        (
            "merge",
            format!(
                "init {{ n: 0 }} step (st,x) -> st merge (a,b) -> {{ n: a.n + {probe} }} finish st -> st.n"
            ),
        ),
        (
            "finish",
            format!(
                "init {{ n: 0 }} step (st,x) -> st merge (a,b) -> a finish st -> st.n + {probe}"
            ),
        ),
    ];
    for (part, body) in parts {
        let s = schema(&format!(
            "aggregate probe (x: Row) -> I64 = state {{ n: I64 }} {body}"
        ));
        let mut want = reads(&s, "left", "Row", &[]);
        want.extend(reads(&s, "right", "Row", &["secret"]));
        let got: BTreeSet<_> = requirements(&s, &[], "probe(x for x in left)")
            .iter()
            .cloned()
            .collect();
        assert_eq!(got, want, "{part}");
        let quiet = body.replace(probe, "0");
        let s = schema(&format!(
            "aggregate probe (x: Row) -> I64 = state {{ n: I64 }} {quiet}"
        ));
        let got: BTreeSet<_> = requirements(&s, &[], "probe(x for x in left)")
            .iter()
            .cloned()
            .collect();
        assert_eq!(got, reads(&s, "left", "Row", &[]), "{part}");
    }
}

#[test]
fn bound_upsert_replacements_bind_new_values_not_the_old_row() {
    let s = schema("");
    let insert = Requirement::Insert(target(&s, "left", "Row"));
    let update = Requirement::UpdateField(
        target(&s, "left", "Row"),
        field(&s, "left", "Row", "public"),
    );
    let text = r#"transact let x = insert left { id: 1, public: "p", secret: "s" } unless conflict on key else update old set Row { old with public: "q" } return x.public"#;
    assert_exact(
        &requirements(&s, &[], text),
        BTreeSet::from([insert.clone(), update.clone()]),
    );
    let text = r#"transact let x = insert left { id: 1, public: "p", secret: "s" } unless conflict on key else update old set Row { old with public: "q" } return x.secret"#;
    let mut want = reads(&s, "left", "Row", &["secret"]);
    want.extend([insert, update]);
    assert_exact(&requirements(&s, &[], text), want);
}

#[test]
fn upsert_old_matches_read_their_arm_constructors_and_charge_only_their_own_edges() {
    let s = schema("");
    let text = r#"insert flow.Draft { id: 1, secret: "s", public: "p" } unless conflict on key else match old | Draft d -> update d set Sent { d with public: "q" } | Sent s -> fail "sent" | Void v -> skip"#;
    let draft = target(&s, "flow", "Draft");
    let mut want = BTreeSet::from([
        Requirement::Insert(draft),
        Requirement::Retag {
            relation: draft.relation,
            from: draft.constructor,
            to: target(&s, "flow", "Sent").constructor,
        },
    ]);
    for ctor in ["Draft", "Sent", "Void"] {
        want.extend(reads(&s, "flow", ctor, &[]));
    }
    assert_exact(&requirements(&s, &[], text), want);
}

#[test]
fn upsert_branches_ignoring_old_read_no_constructor() {
    let s = schema("");
    let insert = Requirement::Insert(target(&s, "flow", "Draft"));
    for otherwise in ["skip", r#"fail "taken""#] {
        let text = format!(
            r#"insert flow.Draft {{ id: 1, secret: "s", public: "p" }} unless conflict on key else {otherwise}"#
        );
        assert_exact(
            &requirements(&s, &[], &text),
            BTreeSet::from([insert.clone()]),
        );
    }
}

#[test]
fn upsert_old_field_and_whole_row_uses_read_as_ranging_does() {
    let s = schema("");
    let proposed =
        r#"insert flow.Draft { id: 1, secret: "s", public: "p" } unless conflict on key"#;
    let mut secret = BTreeSet::from([Requirement::Insert(target(&s, "flow", "Draft"))]);
    let mut whole = secret.clone();
    for ctor in ["Draft", "Sent", "Void"] {
        secret.extend(reads(&s, "flow", ctor, &["secret"]));
    }
    whole.extend(reads(&s, "flow", "Draft", &["secret", "public"]));
    whole.extend(reads(&s, "flow", "Sent", &["secret", "public"]));
    whole.extend(reads(&s, "flow", "Void", &["secret"]));
    for (used, want) in [
        (format!("{proposed} else fail old.secret"), &secret),
        (
            format!("transact let x = {proposed} else skip return x.secret"),
            &secret,
        ),
        (
            format!("transact let x = {proposed} else skip return x"),
            &whole,
        ),
    ] {
        let got: BTreeSet<_> = requirements(&s, &[], &used).iter().cloned().collect();
        assert_eq!(&got, want, "{used}");
    }
}

fn upsert_observing_old(s: &Schema, observe: &str) -> BTreeSet<Requirement> {
    let text = format!(
        r#"insert flow.Draft {{ id: 1, secret: "s", public: "p" }} unless conflict on key else {observe}"#
    );
    requirements(s, &[], &text).iter().cloned().collect()
}

fn ranged_constructors(s: &Schema, ctors: &[&str]) -> BTreeSet<Requirement> {
    let mut want = BTreeSet::from([Requirement::Insert(target(s, "flow", "Draft"))]);
    for ctor in ctors {
        want.extend(reads(s, "flow", ctor, &[]));
    }
    want
}

#[test]
fn upsert_old_tag_reads_every_constructor_it_distinguishes() {
    let s = schema("");
    let observe = r#"match tag(old) | Draft -> skip | Sent -> fail "sent" | Void -> skip"#;
    assert_eq!(
        upsert_observing_old(&s, observe),
        ranged_constructors(&s, &["Draft", "Sent", "Void"])
    );
}

#[test]
fn tag_of_an_old_row_passed_through_a_function_or_alias_reads_its_constructors() {
    let s = schema(
        "fn same (f: Flow) -> Flow = f
fn kind (f: Flow) -> Tag Flow = tag(f)",
    );
    let want = ranged_constructors(&s, &["Draft", "Sent", "Void"]);
    for observe in [
        r#"match tag(same(old)) | Draft -> skip | Sent -> fail "sent" | Void -> skip"#,
        r#"match kind(old) | Draft -> skip | Sent -> fail "sent" | Void -> skip"#,
    ] {
        assert_eq!(upsert_observing_old(&s, observe), want, "{observe}");
    }
    let text = r#"transact let x = insert flow.Draft { id: 1, secret: "s", public: "p" } unless conflict on key else skip return tag(x)"#;
    let got: BTreeSet<_> = requirements(&s, &[], text).iter().cloned().collect();
    assert_eq!(got, want);
}

#[test]
fn constructor_selection_on_an_old_row_reads_the_selected_constructors() {
    let s = schema("");
    let observe = r#"if exists x in [old].Sent where true then fail "sent" else skip"#;
    assert_eq!(
        upsert_observing_old(&s, observe),
        ranged_constructors(&s, &["Sent"])
    );
}

#[test]
fn tag_of_a_ranged_row_reads_as_its_range_does() {
    let s = schema("fn kind (f: Flow) -> Tag Flow = tag(f)");
    let ranged = |ctors: &[&str]| -> BTreeSet<Requirement> {
        ctors
            .iter()
            .flat_map(|c| reads(&s, "flow", c, &[]))
            .collect()
    };
    for (observed, want) in [
        (
            "from x in flow select tag(x)",
            ranged(&["Draft", "Sent", "Void"]),
        ),
        ("from x in flow.Draft select kind(x)", ranged(&["Draft"])),
        (
            "from x in flow.(Draft | Sent) group x by tag(x) into g select g.key",
            ranged(&["Draft", "Sent"]),
        ),
    ] {
        let got: BTreeSet<_> = requirements(&s, &[], observed).iter().cloned().collect();
        assert_eq!(got, want, "{observed}");
    }
}

#[test]
fn tags_of_built_rows_and_parameters_read_nothing() {
    let s = schema("");
    for (params, text) in [
        (&[][..], r#"tag(Draft { id: 1, secret: "s", public: "p" })"#),
        (&[("f", "Flow")][..], "tag(f)"),
        (
            &[("f", "Flow")][..],
            "match tag(f) | Draft -> 1 | Sent -> 2 | Void -> 3",
        ),
        (&[("fs", "Set Flow")][..], "exists x in fs.Sent where true"),
    ] {
        assert_exact(&requirements(&s, params, text), BTreeSet::new());
    }
}

fn every_field(s: &Schema, relations: &[&str]) -> BTreeSet<Requirement> {
    relations
        .iter()
        .flat_map(|r| reads(s, r, "Row", &["public", "secret"]))
        .collect()
}

const TALLY: &str = "aggregate tally (x: Row) -> I64 =
  state { n: I64 } init { n: 0 }
  step (st,x) -> { n: st.n + 1 }
  merge (a,b) -> { n: a.n + b.n }
  finish st -> st.n";

#[test]
fn multiplicity_over_rows_of_two_relations_reads_every_field() {
    let s = schema(TALLY);
    for text in [
        "count(x for x in (left union right))",
        "sum(x.id for x in (left union right))",
        "avg(f64(x.id) for x in (left union right))",
        "tally(x for x in (left union right))",
        "from x in (left union right) group x by x.id into g select count(y for y in g)",
    ] {
        let got: BTreeSet<_> = requirements(&s, &[], text).iter().cloned().collect();
        assert_eq!(got, every_field(&s, &["left", "right"]), "{text}");
    }
}

#[test]
fn multiplicity_over_rebuilt_rows_reads_their_carried_fields() {
    let s = schema("");
    let text = "count(y for y in (from x in left select Row { x with id: 0 }))";
    assert_exact(&requirements(&s, &[], text), every_field(&s, &["left"]));
}

#[test]
fn multiplicity_over_rows_of_one_relation_reads_only_keys() {
    let s = schema(TALLY);
    for text in [
        "count(x for x in left)",
        "count(x for x in (left union left))",
        "sum(x.id for x in left)",
        "tally(x for x in left)",
        "from x in left group x by x.id into g select count(y for y in g)",
        "from y in top 1 by x.id (from x in left select x) select y.id",
    ] {
        let got: BTreeSet<_> = requirements(&s, &[], text).iter().cloned().collect();
        assert_eq!(got, reads(&s, "left", "Row", &[]), "{text}");
    }
    let mut want = reads(&s, "flow", "Draft", &[]);
    want.extend(reads(&s, "flow", "Sent", &[]));
    want.extend(reads(&s, "flow", "Void", &[]));
    assert_exact(&requirements(&s, &[], "count(x for x in flow)"), want);
}

#[test]
fn multiplicity_across_snapshots_of_one_relation_reads_every_field() {
    let s = schema("");
    let params = [("t", "Instant"), ("ts", "Set Instant")];
    for text in [
        "count(x for x in (left union (at instant t { left })))",
        "count(x for p in ts, x in (at instant p { left }))",
    ] {
        let got: BTreeSet<_> = requirements(&s, &params, text).iter().cloned().collect();
        assert_eq!(got, every_field(&s, &["left"]), "{text}");
    }
    for text in [
        "count(x for x in (at instant t { left }))",
        "from p in ts select count(x for x in (at instant p { left }))",
    ] {
        let got: BTreeSet<_> = requirements(&s, &params, text).iter().cloned().collect();
        assert_eq!(got, reads(&s, "left", "Row", &[]), "{text}");
    }
}

const PARTIAL_ALIAS: &str = "transact
  let rows = from y in left where y.id + 1 > 0 select y
  update x in rows set ";

#[test]
fn an_update_through_an_alias_charges_a_field_taken_from_another_row() {
    let s = schema("");
    let text = format!(
        "{PARTIAL_ALIAS}Row {{ x with secret: match max(y.secret for y in rows) | Some m -> m.value | None -> x.secret }}"
    );
    let mut want = reads(&s, "left", "Row", &["secret"]);
    want.insert(Requirement::UpdateField(
        target(&s, "left", "Row"),
        field(&s, "left", "Row", "secret"),
    ));
    assert_exact(&requirements(&s, &[], &text), want);
}

#[test]
fn an_update_through_an_alias_carries_its_own_row_unchanged() {
    let s = schema("");
    let text = format!(r#"{PARTIAL_ALIAS}Row {{ x with public: "p" }}"#);
    let mut want = reads(&s, "left", "Row", &[]);
    want.insert(Requirement::UpdateField(
        target(&s, "left", "Row"),
        field(&s, "left", "Row", "public"),
    ));
    assert_exact(&requirements(&s, &[], &text), want);
}

#[test]
fn consuming_a_stored_row_ranges_its_fieldless_constructors() {
    let s = schema("");
    let program = check_schema(&s).expect("schema checks");
    let normal = crate::norm::program(&s, &program).expect("program normalizes");
    let mut analyzer = Analyzer {
        schema: &s,
        arenas: [&normal.terms, &normal.terms],
        aggregates: &normal.aggregates,
        requirements: Requirements::default(),
        next_id: 0,
        snapshot: 0,
    };
    let flow = RelId(
        s.relations
            .iter()
            .position(|r| r.name == "flow")
            .expect("relation") as u32,
    );
    let void = s
        .ty(s.relation(flow).elem_def)
        .ctor_index("Void")
        .expect("constructor");
    analyzer
        .relation(flow, Some(&[void]), None, false)
        .expect("relation reads");
    assert_exact(&analyzer.requirements, BTreeSet::new());
    let mut row = Value::row(Some(void), BTreeMap::new());
    row.stored.insert(flow);
    analyzer.consume(&row);
    assert_exact(&analyzer.requirements, reads(&s, "flow", "Void", &[]));
    analyzer.requirements = Requirements::default();
    row.stored.clear();
    analyzer.consume(&row);
    assert_exact(&analyzer.requirements, BTreeSet::new());
}

const EXTRA: &str = "type Quota = { id: I64, n: I64, when: Instant }
relation limits : Quota key id
type Pin = { id: I64, draft: Key flow.Draft }
relation pins : Pin key id";

#[test]
fn top_take_and_after_charge_their_counts_and_cursors() {
    let s = schema(EXTRA);
    let text = "from l in limits, y in top l.n by x.id (from x in left select x) select y.public";
    let mut want = reads(&s, "limits", "Quota", &["n"]);
    want.extend(reads(&s, "left", "Row", &["public"]));
    assert_exact(&requirements(&s, &[], text), want);
    let mut want = every_field(&s, &["left"]);
    want.extend(reads(&s, "limits", "Quota", &[]));
    for text in [
        "from x in left select x order by x.id take count(l for l in limits)",
        "from x in left select x order by x.id take 1 after max(l.id for l in limits)",
    ] {
        let got: BTreeSet<_> = requirements(&s, &[], text).iter().cloned().collect();
        assert_eq!(got, want, "{text}");
    }
    let text = "from x in left select x order by x.id take n after c";
    assert_exact(
        &requirements(&s, &[("n", "I64"), ("c", "Option I64")], text),
        every_field(&s, &["left"]),
    );
}

#[test]
fn at_charges_its_point_and_its_body() {
    let s = schema(EXTRA);
    let text = "from l in limits, x in (at instant l.when { left }) select x.public";
    let mut want = reads(&s, "limits", "Quota", &["when"]);
    want.extend(reads(&s, "left", "Row", &["public"]));
    assert_exact(&requirements(&s, &[], text), want);
    let text = "at instant t { from x in left select x.public }";
    assert_exact(
        &requirements(&s, &[("t", "Instant")], text),
        reads(&s, "left", "Row", &["public"]),
    );
}

#[test]
fn try_charges_what_its_body_reads() {
    let s = schema(EXTRA);
    for text in [
        "from x in limits select try (x.id / x.n)",
        "from x in limits select match try (x.id / x.n) | Ok o -> 0 | Err e -> 1",
    ] {
        let got: BTreeSet<_> = requirements(&s, &[], text).iter().cloned().collect();
        assert_eq!(got, reads(&s, "limits", "Quota", &["n"]), "{text}");
    }
    let text = "from x in limits select try x.id";
    assert_exact(
        &requirements(&s, &[], text),
        reads(&s, "limits", "Quota", &[]),
    );
}

#[test]
fn key_of_charges_only_what_follows_it() {
    let s = schema("");
    let text = "from l in links where l.target == left.key(1) select l.id";
    assert_exact(
        &requirements(&s, &[], text),
        reads(&s, "links", "Link", &["target"]),
    );
    let text = "match left.key(1) | Row r -> r.public";
    assert_exact(
        &requirements(&s, &[], text),
        reads(&s, "left", "Row", &["public"]),
    );
}

fn constraint(s: &Schema) -> Requirements {
    let program = check_schema(s).expect("schema checks");
    let normal = crate::norm::program(s, &program).expect("program normalizes");
    analyze(s, &program, &normal.terms, normal.constraints[0], &[]).expect("requirements analyze")
}

#[test]
fn transitions_read_as_their_relation_does() {
    let s =
        schema(r#"constraint marked = from x in inserted(left) where x.secret == "s" select x.id"#);
    assert_exact(&constraint(&s), reads(&s, "left", "Row", &["secret"]));
    let s = schema(
        "constraint kept = from x in deleted(flow.Draft) where x.public == \"p\" select x.id",
    );
    assert_exact(&constraint(&s), reads(&s, "flow", "Draft", &["public"]));
}

#[test]
fn multiplicity_across_a_transition_and_its_relation_reads_every_field() {
    let s = schema(
        "constraint grew = from x in left where count(y for y in (prior(left) union left)) > 5 select x.id",
    );
    assert_exact(&constraint(&s), every_field(&s, &["left"]));
    let s = schema(
        "constraint grew = from x in left where count(y for y in (prior(left) union prior(left))) > 5 select x.id",
    );
    assert_exact(&constraint(&s), reads(&s, "left", "Row", &[]));
}

#[test]
fn dereferencing_a_restricted_key_reads_only_its_constructors() {
    let s = schema(EXTRA);
    let text = "from p in pins select match p.draft | Draft d -> d.public";
    let mut want = reads(&s, "pins", "Pin", &["draft"]);
    want.extend(reads(&s, "flow", "Draft", &["public"]));
    assert_exact(&requirements(&s, &[], text), want);
}
