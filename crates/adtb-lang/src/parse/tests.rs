use super::*;

const STRICT: Options = Options { elisions: false };
const EXAMPLES: Options = Options { elisions: true };

fn expr(src: &str) -> Expr {
    match parse_expr(src, STRICT).ok() {
        Ok(e) => e,
        Err(d) => panic!("{src}: {d:?}"),
    }
}

fn expr_err(src: &str) -> String {
    match parse_expr(src, STRICT).ok() {
        Ok(e) => panic!("{src} parsed as {e:?}"),
        Err(d) => d[0].message.clone(),
    }
}

fn adhoc(src: &str) -> Vec<AdhocItem> {
    match parse_adhoc(src, STRICT).ok() {
        Ok(v) => v,
        Err(d) => panic!("{src}: {d:?}"),
    }
}

fn adhoc_err(src: &str) -> String {
    match parse_adhoc(src, STRICT).ok() {
        Ok(v) => panic!("{src} parsed as {v:?}"),
        Err(d) => d[0].message.clone(),
    }
}

fn example(src: &str) -> Vec<ExampleItem> {
    match parse_example(src, EXAMPLES).ok() {
        Ok(v) => v,
        Err(d) => panic!("{src}: {d:?}"),
    }
}

fn binop(e: &Expr) -> BinOp {
    match &e.kind {
        ExprKind::Binary(op, _, _) => *op,
        k => panic!("not binary: {k:?}"),
    }
}

#[test]
fn precedence_follows_the_table() {
    let e = expr("a or b and not c == d + e * f");
    assert_eq!(binop(&e), BinOp::Or);
    let ExprKind::Binary(_, _, rhs) = &e.kind else {
        unreachable!()
    };
    assert_eq!(binop(rhs), BinOp::And);
    let e = expr("a + b * c");
    assert_eq!(binop(&e), BinOp::Add);
    let e = expr("x union y or z");
    assert_eq!(binop(&e), BinOp::Union);
}

#[test]
fn comparisons_do_not_chain() {
    assert!(expr_err("a < b < c").contains("do not chain"));
}

#[test]
fn negative_literal_is_negation() {
    let e = expr("-3");
    assert!(matches!(e.kind, ExprKind::Unary(UnOp::Neg, _)));
}

#[test]
fn call_needs_an_adjacent_parenthesis() {
    assert!(matches!(expr("f(x)").kind, ExprKind::Call { .. }));
    let items = adhoc("f (x)");
    assert_eq!(items.len(), 2);
}

#[test]
fn method_call_syntax_is_refused() {
    assert!(expr_err("x.f(y)").contains("is a call only"));
}

#[test]
fn key_call_and_type_call() {
    let e = expr("memberships.key(user: u, team: t)");
    assert!(matches!(e.kind, ExprKind::KeyCall(_, KeyArgs::Named(ref n)) if n.len() == 2));
    let e = expr("users.key(3)");
    assert!(matches!(e.kind, ExprKind::KeyCall(_, KeyArgs::Value(_))));
    let e = expr("Instant.parse(t)");
    assert!(matches!(e.kind, ExprKind::TypeCall { .. }));
    let e = expr("(Tag Identity).parse(r.kind)");
    let ExprKind::TypeCall { ty, .. } = &e.kind else {
        panic!("{e:?}")
    };
    assert!(matches!(&ty.kind, TypeKind::Named { args, .. } if args.len() == 1));
}

#[test]
fn reserved_words_are_names_in_field_position() {
    let e = expr("g.key");
    assert!(matches!(e.kind, ExprKind::Field(_, ref f) if f.text == "key"));
    let e = expr("{ key: k, type: t }");
    assert!(matches!(e.kind, ExprKind::Record(RecordExpr::Fields(ref f, _)) if f.len() == 2));
}

#[test]
fn constructor_selection_and_backlinks() {
    assert!(matches!(
        expr("identities.(Google | Apple)").kind,
        ExprKind::Select(_, CtorSel::Many(ref cs)) if cs.len() == 2
    ));
    assert!(matches!(
        expr("u.<identities.Google.user").kind,
        ExprKind::Backlink(_, RelationRef { sel: Some(_), .. }, _)
    ));
}

#[test]
fn constructors_and_records() {
    assert!(matches!(
        expr("Identity.Google { user: u, gid: g }").kind,
        ExprKind::Ctor {
            ctor: Some(_),
            record: Some(_),
            ..
        }
    ));
    assert!(matches!(
        expr("Google { g with gid: g2 }").kind,
        ExprKind::Ctor {
            record: Some(RecordExpr::With(..)),
            ..
        }
    ));
    assert!(matches!(
        expr("None").kind,
        ExprKind::Ctor { record: None, .. }
    ));
    assert!(matches!(expr("[]").kind, ExprKind::Set(ref v) if v.is_empty()));
}

#[test]
fn match_arm_takes_the_following_arms() {
    let e = expr("match a | X -> match b | Y -> 1 | Z -> 2");
    let ExprKind::Match(_, arms) = &e.kind else {
        unreachable!()
    };
    assert_eq!(arms.len(), 1);
    let e = expr("match a | X -> (match b | Y -> 1 | Z -> 2) | W -> 3");
    let ExprKind::Match(_, arms) = &e.kind else {
        unreachable!()
    };
    assert_eq!(arms.len(), 2);
}

#[test]
fn patterns() {
    let e = expr(
        "match i | EmailPass { email, hash: _ } -> email | Google g if g.gid == \"r\" -> 1 | Google g | Apple g -> 2 | _ -> 3",
    );
    let ExprKind::Match(_, arms) = &e.kind else {
        unreachable!()
    };
    assert_eq!(arms.len(), 4);
    assert!(arms[1].guard.is_some());
    assert_eq!(arms[2].patterns.len(), 2);
    let e = expr("match c | \"US\" -> 0 | -1 -> 1 | _ -> 2");
    let ExprKind::Match(_, arms) = &e.kind else {
        unreachable!()
    };
    assert!(matches!(
        arms[1].patterns[0].kind,
        PatternKind::Lit { neg: true, .. }
    ));
}

#[test]
fn comprehension_with_group_and_order() {
    let e = expr(
        "from i in identities, u in users where i.user == key u group i by i.user into g select g.key order by (g.key desc, x) take 50 after c",
    );
    let ExprKind::Comprehension(c) = &e.kind else {
        unreachable!()
    };
    assert_eq!(c.clauses.len(), 2);
    assert!(c.group.is_some());
    let order = c.order.as_ref().unwrap();
    assert_eq!(order.keys.len(), 2);
    assert!(order.keys[0].desc);
    assert!(order.take.is_some() && order.after.is_some());
}

#[test]
fn aggregate_call() {
    let e = expr("count(i for i in identities where i.user == key u)");
    assert!(matches!(
        e.kind,
        ExprKind::Call {
            args: CallArgs::Aggregate {
                filter: Some(_),
                ..
            },
            ..
        }
    ));
}

#[test]
fn fix_binds_several() {
    let e =
        expr("fix a = (from x in s select x), b = (from y in a select y) in from z in b select z");
    assert!(matches!(e.kind, ExprKind::Fix(ref b, _) if b.len() == 2));
}

#[test]
fn top_operand_follows_after_whitespace() {
    let e = expr("top 3 by (p.created_at desc, key p) (from p in posts select p)");
    assert!(matches!(e.kind, ExprKind::Top { ref keys, .. } if keys.len() == 2));
}

#[test]
fn at_and_reject_are_contextual() {
    assert!(matches!(
        expr("at commit p { from i in identities select i }").kind,
        ExprKind::At { instant: false, .. }
    ));
    assert!(matches!(expr("at + 1").kind, ExprKind::Binary(..)));
    assert!(matches!(expr("reject \"bad\"").kind, ExprKind::Reject(_)));
}

#[test]
fn types() {
    let t = parse_type(
        "Set (ByEmail { email: Text } | ByGoogle { gid: Text })",
        STRICT,
    )
    .ok()
    .unwrap();
    let TypeKind::Named { args, .. } = &t.kind else {
        unreachable!()
    };
    assert!(matches!(&args[0], TypeArg::Type(Type { kind: TypeKind::Sum(c), .. }) if c.len() == 2));
    let t = parse_type("Option (Key identities.(Google | Apple))", STRICT)
        .ok()
        .unwrap();
    assert!(matches!(t.kind, TypeKind::Named { ref args, .. } if args.len() == 1));
    let t = parse_type("Identity -> Bool -> Text", STRICT).ok().unwrap();
    let TypeKind::Fn(_, rhs) = &t.kind else {
        unreachable!()
    };
    assert!(matches!(rhs.kind, TypeKind::Fn(..)));
    assert!(parse_type("Decimal 2", STRICT).ok().is_ok());
    assert!(matches!(
        parse_type("Found { email: Text }", STRICT)
            .ok()
            .unwrap()
            .kind,
        TypeKind::Sum(_)
    ));
    assert!(
        parse_type("Result Text { email: Text }", STRICT)
            .ok()
            .is_err()
    );
}

fn type_err(src: &str) -> String {
    match parse_type(src, STRICT).ok() {
        Ok(t) => panic!("{src} parsed as {t:?}"),
        Err(d) => d[0].message.clone(),
    }
}

#[test]
fn prelude_type_before_a_brace_is_applied() {
    for src in [
        "Set { a: I64 }",
        "Option { at: Instant }",
        "Set { a: Set { b: Text } }",
    ] {
        let t = parse_type(src, STRICT).ok().unwrap();
        let TypeKind::Named { name, args, .. } = &t.kind else {
            panic!("{src} parsed as {t:?}")
        };
        assert!(
            name.name.text == "Set" || name.name.text == "Option",
            "{src}"
        );
        assert!(
            matches!(
                &args[..],
                [TypeArg::Type(Type {
                    kind: TypeKind::Record(_),
                    ..
                })]
            ),
            "{src} parsed as {t:?}"
        );
    }
    assert!(matches!(
        parse_type("Result { a: I64 } Text", STRICT).ok().unwrap().kind,
        TypeKind::Named { ref args, .. } if args.len() == 2
    ));
}

#[test]
fn prelude_type_cannot_name_an_anonymous_constructor() {
    for (src, name) in [
        ("Set { a: I64 } | Other", "Set"),
        ("Other | Set { a: I64 }", "Set"),
        ("Found { a: I64 } | Option", "Option"),
        ("Bool | Other", "Bool"),
        ("Uuid { a: I64 } | Other", "Uuid"),
        ("Text | Other", "Text"),
    ] {
        let msg = type_err(src);
        assert_eq!(
            msg,
            format!("`{name}` is a prelude type and cannot name an anonymous constructor"),
            "{src}"
        );
    }
}

#[test]
fn schema_file() {
    let src = r#"
module auth
type User = { id: Key users, name: Text }
type Identity { id: Key identities, user: Key users on delete cascade }
  = EmailPass { email: Text, hash: Bytes }
  | Google    { gid: Text }
relation users : User key id generate sequence
relation identities : Identity
  key id generate uuidv7
  unique (email) in EmailPass
  unique (gid) in Google
  default EmailPass.hash = "x"
set OAuth of identities = Google
fn display_name (i: Identity) -> Text = match i
  | EmailPass e -> e.email
  | Google g    -> g.gid
"#;
    let file = parse_schema(src, STRICT).ok().unwrap();
    assert_eq!(file.decls.len(), 6);
    let Decl::Relation(r) = &file.decls[3] else {
        unreachable!()
    };
    assert_eq!(r.clauses.len(), 4);
}

#[test]
fn schema_file_needs_a_module_header() {
    assert!(parse_schema("type A = B", STRICT).ok().is_err());
}

#[test]
fn a_newtype_base_ends_before_the_next_operator() {
    let src = "module money\nadd type Cents = new I64\nadd type Rate = new I64 where self > 0\nadd relation rates : Rate\n";
    let file = parse_migration(src, STRICT)
        .ok()
        .unwrap_or_else(|d| panic!("{d:?}"));
    assert_eq!(file.operators.len(), 3);
}

#[test]
fn a_newtype_base_ends_before_a_contextual_declaration() {
    let src = "module money\ntype Cents = new I64\naggregate total (x: I64) -> I64 =\n  state { s: I64 } init { s: 0 }\n  step (st, x) -> { s: st.s + x }\n  merge (a, b) -> { s: a.s + b.s }\n  finish st -> st.s\n";
    let file = parse_schema(src, STRICT)
        .ok()
        .unwrap_or_else(|d| panic!("{d:?}"));
    assert_eq!(file.decls.len(), 2);
}

#[test]
fn a_type_argument_on_its_own_line_continues_the_type() {
    let t = parse_type("Box\n  a", STRICT).ok().unwrap();
    assert!(matches!(t.kind, TypeKind::Named { ref args, .. } if args.len() == 1));
}

#[test]
fn migration_operators() {
    let src = r"
module auth
add constructor Identity.Apple { sub: Text }
  joins identities.OAuth
  arm display_name: Apple a -> a.sub
enable identities.Apple
split constructor Identity.Social into Google { gid: Text } | Apple { sub: Text }
  by match
    | Social { provider: Google, sub } -> Google { gid: sub }
    | Social { provider: Apple, sub }  -> Apple { sub: sub }
  unique (gid) in Google
  arm display_name: Google g -> g.gid
move field Identity.EmailPass.verified to Identity fill match
  | Google g -> true
  | Apple a  -> true
rekey relation prices key (sku, during)
  carry Order.Line.price
add default identities.EmailPass.verified = false
grant read identities.EmailPass (user, email) to support
revert 0016
enable Provider.Okta
";
    let file = parse_migration(src, STRICT).ok().unwrap();
    assert_eq!(file.operators.len(), 9);
    let OperatorKind::SplitConstructor { clauses, arms, .. } = &file.operators[2].kind else {
        unreachable!()
    };
    assert_eq!(arms.len(), 2);
    assert_eq!(clauses.len(), 2);
    assert!(matches!(
        file.operators[8].kind,
        OperatorKind::Enable(CtorTarget::Type(_))
    ));
}

#[test]
fn revert_needs_a_migration_number() {
    assert!(parse_migration("module m\nrevert 16", STRICT).ok().is_err());
}

const HASH: &str = "c41e00000000000000000000000000000000000000000000000000000000abcd";

#[test]
fn bundle_file() {
    let src = format!(
        r#"bundle admin-1.0.2 pins 12 ({HASH}) edition "2026-10" lease 90d hide new constructors
import auth
import fn display_name
query list () -> Set User = from u in users select u
txn sign_up (n: Text) -> Key users =
  transact
    let u = insert users {{ name: n }}
    return key u
feed changes of identities.OAuth
"#
    );
    let file = parse_bundle(&src, STRICT).ok().unwrap();
    assert_eq!(file.header.name.family, "admin");
    assert_eq!(file.header.name.version, "1.0.2");
    assert_eq!(file.header.clauses.len(), 2);
    assert_eq!(file.imports.len(), 2);
    assert_eq!(file.items.len(), 3);
}

#[test]
fn bundle_family_may_contain_hyphens() {
    let src = format!(r#"bundle my-app-2.4 pins 1 ({HASH}) edition "2026-10""#);
    let file = parse_bundle(&src, STRICT).ok().unwrap();
    assert_eq!(file.header.name.family, "my-app");
}

#[test]
fn version_hash_is_64_hex_digits() {
    let src = r#"bundle web-1.0 pins 12 (c41e) edition "2026-10""#;
    assert!(parse_bundle(src, STRICT).ok().is_err());
    let src = r#"bundle web-1.0 pins 12 (c41e...) edition "2026-10""#;
    assert!(parse_bundle(src, STRICT).ok().is_err());
    assert_eq!(example(src).len(), 1);
}

#[test]
fn transact_blocks_and_let_bound_writes() {
    let items = adhoc(
        r#"transact retry 5 durability quorum
  let u = insert users { name: n }
  (insert identities.Google { user: key u, gid: g },
   if scalar_len(n) > 0 then skip else fail "empty name")
  return key u"#,
    );
    let [AdhocItem::Transact(b)] = items.as_slice() else {
        panic!("{items:?}")
    };
    assert_eq!(b.options.len(), 2);
    assert_eq!(b.items.len(), 2);
    assert!(matches!(b.items[0], BlockItem::Let(_, LetValue::Write(_))));
    assert!(matches!(
        b.items[1],
        BlockItem::Write(Write {
            kind: WriteKind::Seq(_),
            ..
        })
    ));
    assert!(b.ret.is_some());
}

#[test]
fn upsert() {
    let items = adhoc(
        r"insert identities.Google from (from x in imports select { user: x.user, gid: x.sub })
unless conflict on (gid) in Google
else update old set Google { old with user: new.user }",
    );
    let [
        AdhocItem::Write(Write {
            kind:
                WriteKind::Insert {
                    from: true,
                    upsert: Some(u),
                    ..
                },
            ..
        }),
    ] = items.as_slice()
    else {
        panic!("{items:?}")
    };
    assert!(matches!(u.on, ConflictTarget::Fields(_, Some(_))));
}

#[test]
fn bare_query_after_the_first_item_is_refused() {
    assert!(adhoc_err("insert users { name: n }\ncount(u for u in users)").contains("first item"));
    assert_eq!(
        adhoc("insert users { name: n }\n(count(u for u in users))").len(),
        2
    );
    assert_eq!(
        adhoc("count(u for u in users)\nfrom u in users select u").len(),
        2
    );
}

#[test]
fn commands() {
    let items = adhoc(
        r#"explain analyze web-2.4::login_key(users.key(3))
retire bundle web-2.3
supersede bundle ios-4.1 by ios-4.2
set setting VacuumBudget = { io: 0.2, cpu: 0.5 }
restore from "s3://backups/app" to "restored.adt" before version 12
grant call web::login_key to app
map peer "deploy" on unix to web
alter role web password p
temp relation staged : { email: Text } key email"#,
    );
    assert_eq!(items.len(), 9);
    assert!(matches!(
        items[0],
        AdhocItem::Command(
            Command::Explain {
                analyze: true,
                target: ExplainTarget::Registered { .. }
            },
            _
        )
    ));
    assert!(matches!(
        &items[5],
        AdhocItem::Grant(Grant { privileges, .. })
            if matches!(&privileges[0], Privilege::Call(CallTarget::Family(f, _), Some(_)) if f == "web")
    ));
}

#[test]
fn errors_recover_at_the_next_item() {
    let parsed = parse_schema(
        "module m\ntype A = = B\ntype C = D\nrelation r : = x\nrelation s : C key id",
        STRICT,
    );
    assert_eq!(parsed.diags.len(), 2);
}

#[test]
fn example_mode_mixes_files() {
    let items = example(
        r#"-- 0011
add constructor Identity.Apple { sub: Text }
type Bool ordered = False | True
bundle web-2.3 pins 12 (c41e...) edition "2026-10"
query q () -> Set User = from u in users select u
from u in users select u
Rejected { reason: KeyChanged {..} }"#,
    );
    assert_eq!(items.len(), 6);
}

fn operators(src: &str) -> Vec<OperatorKind> {
    match parse_migration(&format!("module m\n{src}"), STRICT).ok() {
        Ok(f) => f.operators.into_iter().map(|o| o.kind).collect(),
        Err(d) => panic!("{src}: {d:?}"),
    }
}

fn operator_err(src: &str) -> String {
    match parse_migration(&format!("module m\n{src}"), STRICT).ok() {
        Ok(f) => panic!("{src} parsed as {:?}", f.operators),
        Err(d) => d[0].message.clone(),
    }
}

#[test]
fn add_constructor_places_before_or_after() {
    let ops = operators(
        "add constructor Identity.Apple { sub: Text } before Google
add constructor Identity.Okta after EmailPass
  joins identities.OAuth
  arm display_name: Okta o -> o.sub
add constructor Identity.Saml { idp: Text }",
    );
    let [
        OperatorKind::AddConstructor {
            record: Some(_),
            place: Some(CtorPlace::Before(b)),
            ..
        },
        OperatorKind::AddConstructor {
            record: None,
            place: Some(CtorPlace::After(a)),
            joins,
            arms,
            ..
        },
        OperatorKind::AddConstructor { place: None, .. },
    ] = ops.as_slice()
    else {
        panic!("{ops:?}")
    };
    assert_eq!(b.text, "Google");
    assert_eq!(a.text, "EmailPass");
    assert_eq!(joins.len(), 1);
    assert_eq!(arms.len(), 1);
}

#[test]
fn add_constructor_placement_follows_the_record_type() {
    operator_err("add constructor Identity.Apple before Google { sub: Text }");
}

#[test]
fn add_constructor_placement_names_one_constructor() {
    operator_err("add constructor Identity.Apple before identities");
    operator_err("add constructor Identity.Apple after (Google | EmailPass)");
    operator_err("add constructor Identity.Apple before Google after EmailPass");
}

#[test]
fn revert_takes_fills() {
    let ops = operators(
        r#"revert 0015
revert 0015
  fill Identity.EmailPass.hash = bytes("")
revert 0021
  fill Identity.EmailPass.hash = bytes(e.email)
  fill Identity.verified = match
    | EmailPass e -> false
    | Google g    -> true
enable identities.Apple"#,
    );
    let [
        OperatorKind::Revert {
            migration: m0,
            fills: f0,
        },
        OperatorKind::Revert { fills: f1, .. },
        OperatorKind::Revert { fills: f2, .. },
        OperatorKind::Enable(_),
    ] = ops.as_slice()
    else {
        panic!("{ops:?}")
    };
    assert_eq!(m0, "0015");
    assert!(f0.is_empty());
    assert_eq!(f1.len(), 1);
    assert_eq!(f1[0].0.ctor.as_ref().unwrap().text, "EmailPass");
    assert_eq!(f1[0].0.field.text, "hash");
    assert!(matches!(f1[0].1, Fill::Expr(_)));
    assert_eq!(f2.len(), 2);
    assert!(f2[1].0.ctor.is_none());
    assert!(matches!(&f2[1].1, Fill::Match(arms) if arms.len() == 2));
}

#[test]
fn revert_fill_needs_a_field_path_and_a_value() {
    operator_err("revert 0015 fill hash = bytes(\"\")");
    operator_err("revert 0015 fill Identity.EmailPass.hash bytes(\"\")");
    operator_err("revert 0015 fill Identity.EmailPass.hash =");
}

#[test]
fn revert_fill_in_an_example() {
    let items = example(
        r#"-- 0016
module auth
revert 0015
  fill Identity.EmailPass.hash = bytes("")"#,
    );
    assert!(matches!(
        &items[1],
        ExampleItem::Operator(Operator { kind: OperatorKind::Revert { fills, .. }, .. }) if fills.len() == 1
    ));
}

#[test]
fn a_lone_anonymous_constructor_with_a_record_is_a_sum() {
    let t = parse_type("Set (Found { user: Key users })", STRICT)
        .ok()
        .unwrap();
    let TypeKind::Named { args, .. } = &t.kind else {
        unreachable!()
    };
    assert!(matches!(&args[0], TypeArg::Type(Type { kind: TypeKind::Sum(c), .. }) if c.len() == 1));
    assert!(matches!(
        parse_type("Found", STRICT).ok().unwrap().kind,
        TypeKind::Named { .. }
    ));
}

#[test]
fn type_arguments_stop_at_clause_words_and_calls() {
    let file = parse_schema(
        "module m
relation boxes : Box Text unique (label)
relation cells : Cell Text index (lower(label))
relation pins : Box (unique) key id",
        STRICT,
    )
    .ok()
    .unwrap();
    let Decl::Relation(r) = &file.decls[0] else {
        unreachable!()
    };
    assert!(matches!(&r.ty.kind, TypeKind::Named { args, .. } if args.len() == 1));
    assert!(matches!(r.clauses[0], RelationClause::Unique(..)));
    let Decl::Relation(r) = &file.decls[2] else {
        unreachable!()
    };
    assert!(matches!(&r.ty.kind, TypeKind::Named { args, .. } if args.len() == 1));
    let ops = operators("add field Box.size : Option I64 fill None");
    assert!(
        matches!(&ops[0], OperatorKind::AddField { ty: Type { kind: TypeKind::Named { args, .. }, .. }, .. } if args.len() == 1)
    );
}

#[test]
fn top_and_at_are_closed_forms() {
    let e = expr("top 3 by key p (from p in posts select p) union recent");
    assert!(matches!(binop(&e), BinOp::Union));
    let e = expr("at commit p { from i in identities select i } union now");
    assert!(matches!(binop(&e), BinOp::Union));
}

#[test]
fn top_operand_is_a_comprehension() {
    expr_err("top 3 by 1 (xs)");
    expr_err("top 3 by 1 ((from p in posts select p))");
}

#[test]
fn rejects_into_after_an_insert_from_a_query() {
    let items = adhoc(
        r#"insert identities.Google from (
  from r in read_csv("google.csv", GoogleRow)
  select Google { id: r.id, user: r.user, gid: r.gid })
rejects into google_rejects"#,
    );
    assert!(matches!(
        &items[0],
        AdhocItem::Write(Write { kind: WriteKind::Insert { from: true, rejects: Some(r), .. }, .. })
            if r.name.text == "google_rejects"
    ));
    let items = adhoc(
        r"insert identities.Google from (from r in staged select Google { user: r.user, gid: r.gid })
unless conflict on (gid) in Google else skip
rejects into auth::google_rejects",
    );
    assert!(matches!(
        &items[0],
        AdhocItem::Write(Write { kind: WriteKind::Insert { upsert: Some(_), rejects: Some(r), .. }, .. })
            if r.module.is_some()
    ));
}

#[test]
fn let_and_options_that_typing_refuses_still_parse() {
    let items = adhoc(
        r"transact read_only durability local
  let s = skip
  let w = (insert users { name: n }, skip)
  let u = insert users { name: n }
  update u set User { u with name: m }",
    );
    let [AdhocItem::Transact(b)] = items.as_slice() else {
        panic!("{items:?}")
    };
    assert_eq!(b.options.len(), 2);
    assert_eq!(b.items.len(), 4);
}

#[test]
fn text_literals_parse_to_nfc() {
    let composed = expr("\"caf\u{e9}\"");
    let decomposed = expr("\"cafe\u{301}\"");
    let escaped = expr("\"cafe\\u{301}\"");
    assert_eq!(
        composed.kind,
        ExprKind::Lit(Lit::Text("caf\u{e9}".to_string()))
    );
    assert_eq!(decomposed.kind, composed.kind);
    assert_eq!(escaped.kind, composed.kind);
}

#[test]
fn text_patterns_parse_to_nfc() {
    let e = expr("match t | \"cafe\u{301}\" -> 1 | \"cafe\\u{301}\" -> 2 | _ -> 0");
    let ExprKind::Match(_, arms) = &e.kind else {
        unreachable!()
    };
    for arm in &arms[..2] {
        assert_eq!(
            arm.patterns[0].kind,
            PatternKind::Lit {
                neg: false,
                lit: Lit::Text("caf\u{e9}".to_string()),
            }
        );
    }
}
