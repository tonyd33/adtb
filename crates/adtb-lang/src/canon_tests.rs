use super::*;
use crate::evolve::{Source, SourceDecl};
use crate::parse::{Options, parse_migration};
use crate::span::FileId;

fn literal(text: &str) -> String {
    canonical_term(text, Span::new(0, text.len()), &Names::default())
}

fn source(decls: &[(&str, &str)]) -> Source {
    Source::new(
        decls
            .iter()
            .map(|(module, text)| SourceDecl {
                module: module.to_string(),
                text: text.to_string(),
            })
            .collect(),
    )
    .unwrap_or_else(|e| panic!("the schema builds: {e:?}"))
}

/// The canonical token form of declaration `i` of `src` from the first `from` to its end.
fn term(src: &Source, i: usize, from: &str) -> String {
    let text = src.file_text(FileId(i as u32));
    let start = text
        .find(from)
        .unwrap_or_else(|| panic!("no {from:?} in {text:?}"));
    let names = src.names().unwrap_or_else(|e| panic!("names: {e:?}"));
    canonical_term(&text, Span::new(start, text.trim_end().len()), &names[i])
}

/// The canonical token form of the migration text from the first `from` through the first `to` after it.
fn migration_term(src: &Source, text: &str, from: &str, to: &str) -> String {
    let m = parse_migration(text, Options::default())
        .ok()
        .unwrap_or_else(|e| panic!("{text}\ndoes not parse: {e:?}"));
    let after = src
        .migrate(&m, text)
        .unwrap_or_else(|r| panic!("{text}\nrefused: {:?} {:?}", r.operators, r.recheck));
    let names = after
        .log
        .last()
        .expect("the migration is logged")
        .names()
        .unwrap_or_else(|e| panic!("names: {e:?}"));
    let start = text.find(from).expect("the operand is in the text");
    let end = start + text[start..].find(to).expect("the operand ends") + to.len();
    canonical_term(text, Span::new(start, end), &names)
}

const AUTH: [(&str, &str); 3] = [
    ("auth", "type User = { id: Key users, name: Text }"),
    (
        "auth",
        "type Identity { id: Key identities, user: Key users }\n  = Email { address: Text }\n  | Phone { number: Text }",
    ),
    ("auth", "relation users : User key id generate sequence"),
];

fn auth(more: &[(&str, &str)]) -> Source {
    let mut decls = AUTH.to_vec();
    decls.push((
        "auth",
        "relation identities : Identity key id generate uuidv7",
    ));
    decls.extend_from_slice(more);
    source(&decls)
}

#[test]
fn numeric_literals_drop_leading_zeros() {
    assert_eq!(literal("0012"), "12");
    assert_eq!(literal("0"), "0");
    assert_eq!(literal("000"), "0");
    assert_eq!(literal("007.50"), "7.50");
    assert_eq!(literal("00.5"), "0.5");
    assert_eq!(literal("0012ms"), "12ms");
    assert_eq!(literal("0s"), "0s");
    assert_eq!(literal("3min"), "3min");
}

#[test]
fn float_literals_write_a_plain_exponent() {
    assert_eq!(literal("001.50E+003"), "1.50e3");
    assert_eq!(literal("1e-05"), "1e-5");
    assert_eq!(literal("2E0"), "2e0");
    assert_eq!(literal("0.0e00"), "0.0e0");
}

#[test]
fn text_literals_are_their_value_escaped_one_way() {
    assert_eq!(literal(r#""a\u{9}b""#), r#""a\tb""#);
    assert_eq!(literal(r#""q\"\\\u{a}\u{d}""#), r#""q\"\\\n\r""#);
    assert_eq!(literal(r#""\u{e9}\u{263a}""#), "\"\u{e9}\u{263a}\"");
    assert_eq!(literal("\"e\u{301}\""), "\"\u{e9}\"");
}

#[test]
fn whitespace_and_comments_are_dropped() {
    assert_eq!(literal("f(a,   b) -- trailing\n  +   1"), "f ( a , b ) + 1");
    assert_eq!(literal("x.<y"), "x .< y");
}

#[test]
fn the_syntax_example_qualifies_relations() {
    let src = auth(&[(
        "auth",
        "fn logins (u: Key users) -> I64 = count(i for i in identities where i.user == u)",
    )]);
    assert_eq!(
        term(&src, 4, "count"),
        "count ( i for i in auth::identities where i . user == u )"
    );
}

#[test]
fn a_declaration_qualifies_what_it_declares_and_names() {
    let src = auth(&[]);
    assert_eq!(
        term(&src, 1, "type"),
        "type auth::Identity { id : Key auth::identities , user : Key auth::users } = auth::Email { address : Text } | auth::Phone { number : Text }"
    );
    assert_eq!(
        term(&src, 2, "relation"),
        "relation auth::users : auth::User key id generate sequence"
    );
}

#[test]
fn a_user_constructor_is_qualified_and_a_prelude_one_is_not() {
    let src = source(&[
        ("auth", "type Maybe = Nothing | Some { value: I64 }"),
        ("auth", "fn mine () -> Maybe = Some { value: 1 }"),
        ("auth", "fn theirs () -> Option I64 = Some { value: 1 }"),
        (
            "auth",
            "fn both (m: Maybe) -> Option I64 = match m\n  | Some s -> Some { value: s.value }\n  | Nothing -> None",
        ),
    ]);
    assert_eq!(
        term(&src, 1, "fn"),
        "fn auth::mine ( ) -> auth::Maybe = auth::Some { value : 1 }"
    );
    assert_eq!(
        term(&src, 2, "fn"),
        "fn auth::theirs ( ) -> Option I64 = Some { value : 1 }"
    );
    assert_eq!(
        term(&src, 3, "match"),
        "match m | auth::Some s -> Some { value : s . value } | auth::Nothing -> None"
    );
}

#[test]
fn a_name_written_qualified_is_one_token() {
    let src = source(&[
        ("auth", "type User = { id: Key users }"),
        ("auth", "relation users : User key id generate sequence"),
        (
            "billing",
            "fn people () -> I64 = count(u for u in auth::users)",
        ),
    ]);
    assert_eq!(term(&src, 2, "count"), "count ( u for u in auth::users )");
    assert_eq!(
        term(&src, 2, "fn"),
        "fn billing::people ( ) -> I64 = count ( u for u in auth::users )"
    );
}

#[test]
fn a_prelude_name_written_qualified_drops_its_qualifier() {
    let src = source(&[
        (
            "auth",
            "type User = { id: Key users, age: I64, nick: prelude::Option Text }",
        ),
        ("auth", "relation users : User key id generate sequence"),
        (
            "auth",
            "fn nick (u: User) -> prelude::Option I64 = prelude::Option.Some { value: u.age }",
        ),
    ]);
    src.check()
        .unwrap_or_else(|e| panic!("the schema checks: {e:?}"));
    assert_eq!(
        term(&src, 0, "type"),
        "type auth::User = { id : Key auth::users , age : I64 , nick : Option Text }"
    );
    assert_eq!(
        term(&src, 2, "fn"),
        "fn auth::nick ( u : auth::User ) -> Option I64 = Option . Some { value : u . age }"
    );
}

#[test]
fn a_type_named_in_a_body_is_qualified() {
    let src = auth(&[(
        "auth",
        "fn is_email (i: Identity) -> Bool = match i\n  | Identity.Email _ -> true\n  | Identity.Phone _ -> false",
    )]);
    assert_eq!(
        term(&src, 4, "match"),
        "match i | auth::Identity . auth::Email _ -> true | auth::Identity . auth::Phone _ -> false"
    );
}

#[test]
fn a_set_names_its_relation_and_members() {
    let src = auth(&[("auth", "set Reachable of identities = Email | Phone")]);
    assert_eq!(
        term(&src, 4, "set"),
        "set auth::Reachable of auth::identities = auth::Email | auth::Phone"
    );
}

#[test]
fn an_added_type_resolves_in_the_migration_text() {
    let src = auth(&[]);
    let text = "module auth\nadd type Device = { owner: Key users, label: Text }\n";
    assert_eq!(
        migration_term(&src, text, "type Device", "}"),
        "type auth::Device = { owner : Key auth::users , label : Text }"
    );
}

#[test]
fn a_forward_reference_resolves_in_the_result() {
    let src = auth(&[]);
    let text = "module auth\nadd type Device = { id: Key devices, owner: Key users }\nadd relation devices : Device key id generate sequence\n";
    assert_eq!(
        migration_term(&src, text, "type Device", "}"),
        "type auth::Device = { id : Key auth::devices , owner : Key auth::users }"
    );
}

#[test]
fn a_fill_resolves_as_its_operator_checks_it() {
    let src = source(&[
        ("auth", "type Level = Low | High"),
        ("auth", "type User = { id: Key users, name: Text }"),
        ("auth", "relation users : User key id generate sequence"),
    ]);
    let text = "module auth\nadd field User.level: Level fill High\n";
    assert_eq!(
        migration_term(&src, text, "Level fill", "Level"),
        "auth::Level"
    );
    assert_eq!(migration_term(&src, text, "High", "High"), "auth::High");
}

#[test]
fn an_added_constructor_and_its_arms_resolve_where_they_are_written() {
    let src = auth(&[(
        "auth",
        "fn label (i: Identity) -> Text = match i\n  | Email e -> e.address\n  | Phone p -> p.number",
    )]);
    let text = "module auth\nadd constructor Identity.Passkey { key: Bytes, level: Option I64 }\n  arm label: Passkey k -> \"passkey\"\n";
    assert_eq!(
        migration_term(&src, text, "Passkey {", "}"),
        "auth::Passkey { key : Bytes , level : Option I64 }"
    );
    assert_eq!(
        migration_term(&src, text, "Passkey k", "\"passkey\""),
        "auth::Passkey k -> \"passkey\""
    );
}

#[test]
fn a_function_redefined_resolves_its_new_body() {
    let src = auth(&[(
        "auth",
        "fn logins (u: Key users) -> I64 = count(i for i in identities where i.user == u)",
    )]);
    let text = "module auth\nredefine fn logins (u: Key users) -> I64 = count(i for i in identities where i.user == u and match i | Email _ -> true | Phone _ -> false)\n";
    assert_eq!(
        migration_term(&src, text, "fn logins", ")\n"),
        "fn auth::logins ( u : Key auth::users ) -> I64 = count ( i for i in auth::identities where i . user == u and match i | auth::Email _ -> true | auth::Phone _ -> false )"
    );
}

fn oauth() -> Source {
    source(&[
        ("ext", "type Provider = Google | Apple"),
        ("auth", "type User = { id: Key users, name: Text }"),
        (
            "auth",
            "type Identity { id: Key identities, user: Key users }\n  = EmailPass { email: Text } | OAuth { provider: ext::Provider, sub: Text }",
        ),
        ("auth", "relation users : User key id generate sequence"),
        (
            "auth",
            "relation identities : Identity key id generate uuidv7 unique (provider, sub) in OAuth",
        ),
        (
            "auth",
            "fn display_name (i: Identity) -> Text = match i\n  | EmailPass e -> e.email\n  | OAuth o -> o.sub",
        ),
    ])
}

#[test]
fn a_routing_resolves_constructors_by_the_type_it_checks_them_at() {
    let text = "module auth
split constructor Identity.OAuth into Google { gid: Text } | Apple { sub: Text }
  by match
    | OAuth { provider: Google, sub } -> Google { gid: sub }
    | OAuth { provider: Apple, sub }  -> Apple { sub: sub }
  unique (gid) in Google
  unique (sub) in Apple
  arm display_name: Google g -> g.gid
  arm display_name: Apple a -> a.sub
";
    assert_eq!(
        migration_term(&oauth(), text, "Google {", "}"),
        "auth::Google { gid : Text }"
    );
    assert_eq!(
        migration_term(&oauth(), text, "OAuth { provider: Google", "gid: sub }"),
        "auth::OAuth { provider : ext::Google , sub } -> auth::Google { gid : sub }"
    );
    assert_eq!(
        migration_term(&oauth(), text, "Apple a", "a.sub"),
        "auth::Apple a -> a . sub"
    );
}

#[test]
fn a_dropped_unique_resolves_in_the_clause_it_removes() {
    let text = "module auth\ndrop unique (provider, sub) in identities.OAuth\n";
    assert_eq!(
        migration_term(&oauth(), text, "provider,", "sub"),
        "provider , sub"
    );
}
