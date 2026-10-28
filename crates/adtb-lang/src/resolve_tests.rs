use super::*;
use crate::parse::{Options, parse_adhoc, parse_schema};
use crate::schema::build;

const RUNNING: &str = r"
module auth
type User = { id: Key users, name: Text }
type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes }
  | Google    { gid: Text }
relation users : User key id generate sequence
relation identities : Identity
  key id generate uuidv7
  unique (email) in EmailPass
  unique (gid) in Google
set OAuth of identities = Google
fn display_name (i: Identity) -> Text = match i
  | EmailPass e -> e.email
  | Google g    -> g.gid
";

fn schema_errors(extra: &str) -> Vec<String> {
    let src = format!("{RUNNING}\n{extra}");
    let file = parse_schema(&src, Options::default()).ok().unwrap();
    match build(&[(FileId(0), file)]) {
        Ok(_) => Vec::new(),
        Err(ds) => ds.into_iter().map(|d| d.diag.message).collect(),
    }
}

fn accepts(extra: &str) {
    let errs = schema_errors(extra);
    assert!(errs.is_empty(), "{extra}\n{errs:?}");
}

fn refuses(extra: &str, needle: &str) {
    let errs = schema_errors(extra);
    assert!(
        errs.iter().any(|e| e.contains(needle)),
        "{extra}\nno error containing {needle:?} in {errs:?}"
    );
}

#[test]
fn binders_scope_over_later_clauses() {
    accepts(
        "fn f () -> Set Text = from u in users, i in identities where i.user == key u select u.name",
    );
    refuses(
        "fn f () -> Set Text = from u in users where i.user == key u select u.name",
        "no `i` in scope",
    );
}

#[test]
fn group_hides_earlier_binders() {
    accepts(
        "fn f () -> Set I64 = from i in identities group i by i.user into g select count(x for x in g)",
    );
    refuses(
        "fn f () -> Set I64 = from i in identities group i by i.user into g select i.user",
        "no `i` in scope",
    );
}

#[test]
fn parameters_and_caller() {
    accepts(
        "fn f (u: Key users) -> Set Text = from i in identities where i.user == u select display_name(i)",
    );
    accepts("fn me () -> Principal = caller");
    refuses(
        "constraint c = from u in users where caller == caller select u",
        "`caller` is not in scope",
    );
}

#[test]
fn unknown_names_and_functions() {
    refuses("fn f () -> I64 = nope", "no `nope` in scope");
    refuses("fn f () -> I64 = nope(1)", "no `nope` in scope");
    accepts("fn f () -> I64 = scalar_len(\"x\")");
    refuses("fn f () -> I64 = scalar_len", "no `scalar_len` in scope");
}

#[test]
fn match_arms_bind_their_patterns() {
    accepts(
        "fn f (i: Identity) -> Text = match i | EmailPass { email, hash: _ } -> email | Google g -> g.gid",
    );
    refuses(
        "fn f (i: Identity) -> Text = match i | EmailPass e -> g.gid | Google g -> g.gid",
        "no `g` in scope",
    );
    refuses(
        "fn f (i: Identity) -> Key users = match i | EmailPass e | Google g -> e.user",
        "binds the same names",
    );
    refuses(
        "fn f (i: Identity) -> I64 = match i | Identity.Apple a -> 1",
        "not a constructor of `Identity`",
    );
}

#[test]
fn checks_see_the_row_fields() {
    accepts("check email_length on identities.EmailPass (scalar_len(email) <= 254)");
    refuses(
        "check bad on identities (scalar_len(email) <= 254)",
        "no `email` in scope",
    );
}

#[test]
fn backlinks_name_a_field_of_the_relation() {
    accepts("fn f () -> Set User = from u in users where exists i in u.<identities.user select u");
    refuses(
        "fn f () -> Set User = from u in users where exists i in u.<identities.owner select u",
        "not a field",
    );
}

#[test]
fn key_calls_follow_a_relation() {
    accepts("fn f () -> Key users = users.key(3)");
    refuses(
        "fn f (x: I64) -> Key users = x.key(3)",
        "follows a relation name",
    );
}

#[test]
fn newtype_calls() {
    accepts("type Email = new Text\nfn f () -> Email = Email(\"a@b\")");
    refuses("fn f () -> User = User(\"a\")", "not a newtype");
}

#[test]
fn top_keys_see_the_operand_select() {
    accepts(
        "fn f (u: Key users) -> Set Identity = top 3 by (key i) (from i in identities where i.user == u select i)",
    );
}

#[test]
fn fix_binders_are_mutually_visible() {
    accepts(
        "fn f () -> Set (Key users) = fix a = (from u in users select key u) union b, b = (from x in a select x) in from y in a select y",
    );
}

fn adhoc(src: &str) -> Result<BTreeSet<String>, Vec<String>> {
    let file = parse_schema(RUNNING, Options::default()).ok().unwrap();
    let schema = build(&[(FileId(0), file)]).unwrap();
    let items = parse_adhoc(src, Options::default()).ok().unwrap();
    check_adhoc(&schema, &items).map_err(|ds| ds.into_iter().map(|d| d.message).collect())
}

#[test]
fn adhoc_free_names_are_parameters() {
    let params = adhoc("from i in identities where i.user == u select i").unwrap();
    assert_eq!(
        params.into_iter().collect::<Vec<_>>(),
        vec!["u".to_string()]
    );
    assert!(adhoc("from i in identities select frob(i)").is_err());
}

#[test]
fn writes_bind_rows() {
    adhoc("update g in identities.Google where g.user == u set Google { g with gid: g2 }").unwrap();
    assert!(adhoc("update g set Google { g with gid: g2 }").is_err());
    adhoc(
        "insert identities.Google { user: u, gid: g } unless conflict on (gid) in Google else update old set Google { old with user: new.user }",
    )
    .unwrap();
    adhoc("transact\n  let u = insert users { name: n }\n  return key u").unwrap();
    assert!(adhoc("insert nowhere { name: n }").is_err());
}

#[test]
fn root_principal_resolves_from_empty_imports_and_the_prelude() {
    let file = parse_schema(
        "module identity\ntype Principal = EndUser { level: I8 } | Service { name: Text } | Anonymous",
        Options::default(),
    )
    .ok()
    .unwrap();
    for custom in [false, true] {
        let files = if custom {
            vec![(FileId(0), file.clone())]
        } else {
            Vec::new()
        };
        let schema = build(&files).unwrap();
        let name = QName {
            module: None,
            name: Ident {
                text: "Principal".into(),
                span: Span::default(),
            },
        };
        let expected = Ok(TypeName::Decl(schema.principal));
        assert_eq!(expected, schema.lookup_type(crate::schema::PRELUDE, &name));
        for vis in [
            Visible {
                modules: Vec::new(),
            },
            Visible::all(&schema),
            Visible::module(crate::schema::PRELUDE),
        ] {
            assert_eq!(
                expected,
                lookup_type(&schema, &vis, &name),
                "custom={custom}, {vis:?}"
            );
        }
        if custom {
            let qualified = QName {
                module: Some(Ident {
                    text: "identity".into(),
                    span: Span::default(),
                }),
                ..name
            };
            assert_eq!(
                Err("module `identity` declares no type `Principal`".into()),
                lookup_type(
                    &schema,
                    &Visible {
                        modules: Vec::new()
                    },
                    &qualified
                )
            );
        } else {
            assert_eq!(schema.principal, schema.prelude_type("Principal"));
            assert_eq!(37, schema.ids.types[&schema.principal]);
        }
    }
}
