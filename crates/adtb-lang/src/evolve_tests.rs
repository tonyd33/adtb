use super::*;
use crate::parse::parse_migration;

const RUNNING: &str = r"
module auth
type User = { id: Key users, name: Text }
type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes }
  | Google    { gid: Text }
  | Apple     { sub: Text }
relation users : User key id generate sequence
relation identities : Identity
  key id generate uuidv7
  unique (email) in EmailPass
  unique (gid) in Google
  unique (sub) in Apple
set OAuth of identities = Google | Apple
fn display_name (i: Identity) -> Text = match i
  | EmailPass e -> e.email
  | Google g    -> g.gid
  | Apple a     -> a.sub
constraint login_present =
  from i in identities
  where match i
    | EmailPass e -> scalar_len(e.email) == 0
    | Google g    -> scalar_len(g.gid) == 0
    | Apple a     -> scalar_len(a.sub) == 0
  select i
";

/// The source of a schema text: one module header, then declarations.
fn source(text: &str) -> Source {
    let items = crate::parse::parse_example(text, Options::default())
        .ok()
        .expect("the schema parses");
    let mut module = String::new();
    let mut decls = Vec::new();
    for item in items {
        match item {
            ast::ExampleItem::Module(m) => module = m.text,
            ast::ExampleItem::Decl(d) => {
                let span = match &d {
                    Decl::Type(t) => t.span,
                    Decl::Relation(r) => r.span,
                    Decl::Set(s) => s.span,
                    Decl::Fn(f) => f.span,
                    Decl::Constraint(c) => c.span,
                    Decl::Check(c) => c.span,
                    Decl::Store(s) => s.span,
                    Decl::Aggregate(a) => a.span,
                };
                decls.push(SourceDecl {
                    module: module.clone(),
                    text: text[span.range()].to_string(),
                });
            }
            _ => panic!("a schema holds only declarations"),
        }
    }
    Source::new(decls).unwrap_or_else(|e| panic!("the schema builds: {e:?}"))
}

fn running() -> Source {
    source(RUNNING)
}

/// Applies the migration text to `src`.
fn migrate(src: &Source, text: &str) -> Result<Source, Vec<String>> {
    let m = parse_migration(text, Options::default())
        .ok()
        .unwrap_or_else(|e| panic!("{text}\ndoes not parse: {e:?}"));
    src.migrate(&m, text).map_err(|r| {
        let mut out: Vec<String> = r.operators.into_iter().map(|d| d.message).collect();
        out.extend(r.recheck.into_iter().map(|d| d.diag.message));
        out
    })
}

fn applies(src: &Source, text: &str) -> Source {
    let next = migrate(src, text)
        .unwrap_or_else(|e| panic!("{text}\nrefused: {e:?}\n{}", printed_or(src)));
    reprints(&next, text);
    next
}

/// Asserts that each declaration `src` prints parses as one declaration, and that the printed
/// declarations build and check as a schema of their own.
fn reprints(src: &Source, text: &str) {
    for d in &src.decls {
        let file = format!("{}{}", header(&d.module), d.text);
        let parsed = parse_schema(&file, Options { elisions: true })
            .ok()
            .unwrap_or_else(|e| {
                panic!("{text}\nprints a declaration that does not parse: {e:?}\n{file}")
            });
        assert_eq!(
            1,
            parsed.decls.len(),
            "{text}\nprints more than one declaration as one:\n{file}"
        );
    }
    let fresh = Source::new(src.decls.clone()).unwrap_or_else(|e| {
        panic!(
            "{text}\nprints a schema that does not build: {e:?}\n{}",
            printed_or(src)
        )
    });
    if let Err(e) = fresh.check() {
        panic!(
            "{text}\nprints a schema that does not check: {e:?}\n{}",
            printed_or(src)
        );
    }
}

fn printed_or(src: &Source) -> String {
    src.decls
        .iter()
        .map(|d| format!("[{}] {}", d.module, d.text))
        .collect::<Vec<_>>()
        .join("\n")
}

fn refuses(src: &Source, text: &str, needle: &str) {
    match migrate(src, text) {
        Ok(s) => panic!(
            "{text}\napplied; expected a refusal containing {needle:?}\n{}",
            printed_or(&s)
        ),
        Err(errs) => assert!(
            errs.iter().any(|e| e.contains(needle)),
            "{text}\nno refusal containing {needle:?} in {errs:?}"
        ),
    }
}

/// The text of the declaration of `kind` `name` in module `auth`.
fn decl(src: &Source, kind: &str, name: &str) -> String {
    let i = src
        .find(kind, "auth", name)
        .unwrap_or_else(|| panic!("no {kind} `{name}` in\n{}", printed_or(src)));
    src.decls[i].text.clone()
}

/// The declaration's text with its white space collapsed.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn stage(src: &Source, relation: &str, ctor: &str) -> Stage {
    src.states[&("auth".to_string(), relation.to_string(), ctor.to_string())].stage
}

const BADGES: &str = r"
module auth
type Badge { name: Text } = Plain | Color { rgb: I32 }
relation post_badges : Badge key name
relation user_badges : Badge key name
set Simple of post_badges = Plain
";

fn with_badges() -> Source {
    source(&format!(
        "{RUNNING}{}",
        BADGES.trim_start_matches("\nmodule auth")
    ))
}

#[test]
fn a_new_source_has_every_constructor_enabled() {
    let s = running();
    assert_eq!(Stage::Enabled, stage(&s, "identities", "Apple"));
    assert_eq!(Stage::Enabled, stage(&s, "users", "User"));
    assert_eq!(0, s.version);
}

#[test]
fn a_migration_takes_the_next_version_and_joins_the_log() {
    let s = applies(&running(), "module auth\nadd index (user) in identities");
    assert_eq!(1, s.version);
    assert_eq!(1, s.log.len());
    assert_eq!(1, s.log[0].number);
}

#[test]
fn a_refused_migration_lists_every_failure() {
    let errs = migrate(
        &running(),
        "module auth\ndrop fn nosuch\ndrop constraint nothing",
    )
    .unwrap_err();
    assert!(
        errs.iter().any(|e| e.contains("no fn `nosuch`")),
        "{errs:?}"
    );
    assert!(
        errs.iter().any(|e| e.contains("no constraint `nothing`")),
        "{errs:?}"
    );
}

#[test]
fn modules_are_added_and_dropped_empty() {
    let s = applies(&running(), "module billing\nadd module billing");
    assert!(s.modules.contains("billing"));
    let s = applies(&s, "module billing\ndrop module billing");
    assert!(!s.modules.contains("billing"));
    refuses(
        &running(),
        "module auth\nadd module auth",
        "module `auth` already exists",
    );
    refuses(
        &running(),
        "module x\nadd module catalog",
        "no module may be named `catalog`",
    );
    refuses(
        &running(),
        "module x\nadd module sys",
        "no module may be named `sys`",
    );
    refuses(
        &running(),
        "module x\nadd module prelude",
        "no module may be named `prelude`",
    );
    refuses(
        &running(),
        "module auth\ndrop module auth",
        "`drop module auth` is refused while the module holds type `User`",
    );
    refuses(
        &running(),
        "module x\ndrop module nosuch",
        "no module `nosuch`",
    );
}

#[test]
fn a_migration_of_a_missing_module_adds_it_first() {
    refuses(
        &running(),
        "module billing\nadd type Invoice = { id: Key invoices }",
        "no module `billing`",
    );
    let s = applies(
        &running(),
        "module billing\nadd module billing\nadd type Invoice = { id: Key invoices }\nadd relation invoices : Invoice key id generate sequence",
    );
    assert!(s.find("type", "billing", "Invoice").is_some());
}

const WORLD: &str = "module auth
type Region { id: Key regions, parent: Option (Key regions.Settled) on delete clear }
  = Wild
  | Town { mayor: Key people.Living }
type Person { id: Key people, home: Key regions.Town } = Living | Dead
relation regions : Region key id generate sequence
set Settled of regions = Town
relation people : Person key id generate sequence
fn mayor_of (t: Region.Town) -> Key people.Living = t.mayor
constraint mayors_live_at_home =
  from t in regions.Town, p in people.Living
  where t.mayor == key p and p.home != key t
  select t";

/// The first migration of `src`'s module that declares what `src` prints, each declaration
/// prefixed by its verb.
fn replay(src: &Source, module: &str) -> String {
    let mut out = format!("module {module}\nadd module {module}\n");
    for (i, d) in src.decls.iter().enumerate() {
        let verb = match src.parsed(i) {
            Some(Decl::Fn(_) | Decl::Aggregate(_)) => "define ",
            Some(Decl::Store(_)) => "",
            _ => "add ",
        };
        out.push_str(&format!("{verb}{}\n", d.text));
    }
    out
}

#[test]
fn a_reference_selects_constructors_of_a_relation_added_later_with_its_type() {
    let s = applies(
        &Source::default(),
        "module world
add module world
add type Region { id: Key regions, parent: Option (Key regions.Settled) }
  = Wild
  | Town { mayor: Key people.Living }
add type Person { id: Key people, home: Key regions.Town }
  = Living
  | Dead
add relation regions : Region key id generate sequence
add set Settled of regions = Town
add relation people : Person key id generate sequence",
    );
    let schema = s.schema().expect("the schema builds");
    let region = ty(&schema, "Region");
    let parent = &schema.ty(region).header()[1];
    assert_eq!(
        "Option (Key regions.Town)",
        schema.show_ty(&parent.ty),
        "{parent:?}"
    );
}

#[test]
fn a_printed_schema_replays_as_a_first_migration() {
    let printed = source(&format!(
        "{RUNNING}{}",
        WORLD.trim_start_matches("module auth")
    ));
    let s = applies(&Source::default(), &replay(&printed, "auth"));
    let texts = |s: &Source| s.decls.iter().map(|d| d.text.clone()).collect::<Vec<_>>();
    assert_eq!(texts(&printed), texts(&s));
}

#[test]
fn a_late_selection_needs_the_type_added_in_the_same_migration() {
    let s = applies(
        &running(),
        "module auth\nadd type Seat { n: I64 } = Free | Taken",
    );
    refuses(
        &s,
        "module auth
add type Holder = { id: Key holders, seat: Key seats.Free }
add relation holders : Holder key id generate sequence
add relation seats : Seat key n",
        "its type is not added by this migration, so only `Key seats` may name it",
    );
    applies(
        &s,
        "module auth
add type Holder = { id: Key holders, seat: Key seats }
add relation holders : Holder key id generate sequence
add relation seats : Seat key n",
    );
}

#[test]
fn a_late_selection_that_never_resolves_is_refused_when_the_migration_ends() {
    refuses(
        &Source::default(),
        "module auth
add module auth
add type Region { id: Key regions, parent: Option (Key regions.Nowhere) } = Wild | Town
add relation regions : Region key id generate sequence",
        "`Nowhere` is neither a constructor of `Region` nor a named set of `regions`",
    );
}

#[test]
fn extensions_install_and_uninstall() {
    let s = applies(
        &running(),
        "module auth\ninstall geo \"1.2\" from \"geo-1.2.adtx\"",
    );
    assert!(s.installed.contains("geo"));
    let t = applies(&s, "module auth\nuninstall geo");
    assert!(!t.modules.contains("geo"));
    refuses(
        &running(),
        "module auth\nuninstall geo",
        "`geo` is not an installed extension",
    );
    let mut used = s;
    used.decls.push(SourceDecl {
        module: "auth".into(),
        text: "fn near (x: F64) -> F64 = geo::scale(x)".into(),
    });
    refuses(
        &used,
        "module auth\nuninstall geo",
        "`uninstall geo` is refused while fn `near` names it",
    );
}

#[test]
fn types_relations_and_sets_are_added_with_the_declaration_syntax() {
    let s = applies(
        &running(),
        "module auth
add type Device = { id: Key devices, owner: Key users on delete restrict, label: Text }
add relation devices : Device key id generate sequence
add set Mail of identities = EmailPass",
    );
    assert_eq!(
        "type Device = { id: Key devices, owner: Key users on delete restrict, label: Text }",
        decl(&s, "type", "Device")
    );
    assert!(s.find("set", "auth", "identities.Mail").is_some());
    assert_eq!(Stage::Enabled, stage(&s, "devices", "Device"));
}

#[test]
fn an_added_declaration_must_check() {
    refuses(
        &running(),
        "module auth\nadd type Bad = { id: Key nowhere }",
        "nowhere",
    );
    refuses(
        &running(),
        "module auth\nadd type User = { id: Key users }",
        "already declares type `User`",
    );
    refuses(
        &running(),
        "module auth\nadd relation sys::mine : User key id",
        "a relation name is qualified only in the engine's own declarations",
    );
    refuses(
        &running(),
        "module auth\nadd relation mine : User key id volatile",
        "`volatile` appears only in the engine's own declarations",
    );
}

#[test]
fn drop_type_is_refused_while_anything_uses_it() {
    refuses(
        &running(),
        "module auth\ndrop type Identity",
        "`drop type Identity` is refused while relation `identities` uses it",
    );
    let s = applies(&running(), "module auth\nadd type Spare = A | B");
    let s = applies(&s, "module auth\ndrop type Spare");
    assert!(s.find("type", "auth", "Spare").is_none());
    let s = applies(
        &running(),
        "module auth\nadd type Tier = Free | Paid\nadd type Plan = { id: Key plans, tier: Tier }\nadd relation plans : Plan key id generate sequence",
    );
    refuses(
        &s,
        "module auth\ndrop type Tier",
        "while field `Plan.tier` uses it",
    );
    let s = applies(
        &running(),
        "module auth\nadd type Tier = Free | Paid\ndefine fn free () -> Tier = Free",
    );
    refuses(
        &s,
        "module auth\ndrop type Tier",
        "while function `free` uses it",
    );
}

#[test]
fn drop_relation_is_refused_while_a_reference_or_definition_names_it() {
    refuses(
        &running(),
        "module auth\ndrop relation users",
        "`drop relation users` is refused while field `Identity.user` references it",
    );
    refuses(
        &running(),
        "module auth\ndrop relation identities",
        "`drop relation identities` is refused while constraint `login_present` names it",
    );
    let s = with_badges();
    let s = applies(&s, "module auth\ndrop relation post_badges");
    assert!(s.find("relation", "auth", "post_badges").is_none());
    assert!(s.find("set", "auth", "post_badges.Simple").is_none());
    assert!(
        !s.states
            .contains_key(&("auth".into(), "post_badges".into(), "Plain".into()))
    );
}

#[test]
fn sets_are_dropped_and_renamed() {
    let s = applies(
        &running(),
        "module auth\nrename set OAuth of identities to Federated",
    );
    assert!(s.find("set", "auth", "identities.Federated").is_some());
    refuses(
        &running(),
        "module auth\nrename set OAuth of identities to Google",
        "`identities` already has a constructor or named set `Google`",
    );
    let s = applies(&running(), "module auth\ndrop set OAuth of identities");
    assert!(s.find("set", "auth", "identities.OAuth").is_none());
    refuses(
        &running(),
        "module auth\ndrop set Nope of identities",
        "declares no named set `Nope`",
    );
}

#[test]
fn rename_set_rewrites_the_scopes_and_references_naming_it() {
    let s = applies(
        &running(),
        "module auth
add index (user) in identities.OAuth
add type Link = { id: Key links, login: Key identities.OAuth }
add relation links : Link key id generate sequence
define fn federated () -> Set Text = from i in identities.OAuth select match i | Google g -> g.gid | Apple a -> a.sub",
    );
    let s = applies(
        &s,
        "module auth\nrename set OAuth of identities to Federated",
    );
    assert!(decl(&s, "relation", "identities").contains("index (user) in Federated"));
    assert!(decl(&s, "type", "Link").contains("Key identities.Federated"));
    assert!(decl(&s, "fn", "federated").contains("identities.Federated"));
}

#[test]
fn renames_rewrite_every_object_that_names_the_renamed_one() {
    let s = applies(
        &running(),
        "module auth
rename constructor Identity.Google to Gmail
rename type Identity to Login
rename relation identities to logins
rename set OAuth of logins to Federated
rename field Login.EmailPass.hash to pw_hash",
    );
    let t = flat(&decl(&s, "type", "Login"));
    assert!(
        t.starts_with("type Login { id: Key logins, user: Key users }"),
        "{t}"
    );
    assert!(t.contains("| Gmail { gid: Text }"), "{t}");
    assert!(
        t.contains("EmailPass { email: Text, pw_hash: Bytes }"),
        "{t}"
    );
    let r = flat(&decl(&s, "relation", "logins"));
    assert!(r.starts_with("relation logins : Login"), "{r}");
    assert!(r.contains("unique (gid) in Gmail"), "{r}");
    assert_eq!(
        "set Federated of logins = Gmail | Apple",
        decl(&s, "set", "logins.Federated")
    );
    let f = flat(&decl(&s, "fn", "display_name"));
    assert!(f.starts_with("fn display_name (i: Login)"), "{f}");
    assert!(f.contains("| Gmail g -> g.gid"), "{f}");
    let c = flat(&decl(&s, "constraint", "login_present"));
    assert!(c.contains("from i in logins"), "{c}");
    assert!(c.contains("| Gmail g -> scalar_len(g.gid) == 0"), "{c}");
    assert_eq!(Stage::Enabled, stage(&s, "logins", "Gmail"));
}

#[test]
fn a_rename_is_refused_when_the_new_name_is_taken() {
    refuses(
        &running(),
        "module auth\nrename type Identity to User",
        "already declares type `User`",
    );
    refuses(
        &running(),
        "module auth\nrename relation identities to users",
        "already declares `users`",
    );
    refuses(
        &running(),
        "module auth\nrename constructor Identity.Google to Apple",
        "`Apple` is already a constructor of `Identity`",
    );
    refuses(
        &running(),
        "module auth\nrename field Identity.EmailPass.hash to email",
        "`Identity.EmailPass` already has a field `email`",
    );
    refuses(
        &running(),
        "module auth\nrename field Identity.EmailPass.hash to user",
        "already has a field `user`",
    );
}

#[test]
fn rename_field_follows_accesses_patterns_and_records() {
    let s = applies(
        &running(),
        "module auth
define fn gids () -> Set Text = from g in identities.Google select g.gid
define fn pun (i: Identity) -> Text = match i | Google { gid } -> gid | EmailPass e -> e.email | Apple a -> a.sub
define fn make (u: Key users) -> Identity = Google { id: identities.key(uuid(\"x\")), user: u, gid: \"g\" }",
    );
    let s = applies(
        &s,
        "module auth\nrename field Identity.Google.gid to subject",
    );
    assert!(decl(&s, "fn", "gids").contains("g.subject"));
    assert!(decl(&s, "fn", "pun").contains("Google { subject: gid } -> gid"));
    assert!(decl(&s, "fn", "make").contains("subject: \"g\""));
    assert!(decl(&s, "fn", "display_name").contains("g.subject"));
    assert!(decl(&s, "relation", "identities").contains("unique (subject) in Google"));
    assert!(decl(&s, "fn", "display_name").contains("e.email"));
}

#[test]
fn a_header_field_renames_with_its_keys() {
    let s = applies(&running(), "module auth\nrename field Identity.id to ident");
    assert!(decl(&s, "type", "Identity").contains("ident: Key identities"));
    assert!(decl(&s, "relation", "identities").contains("key ident generate uuidv7"));
}

#[test]
fn add_constructor_appends_joins_and_gives_arms() {
    let s = applies(
        &running(),
        "module auth
add constructor Identity.Corp { domain: Text }
  joins identities.OAuth
  arm display_name: Corp c -> c.domain
  arm login_present: Corp c -> scalar_len(c.domain) == 0",
    );
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(
        t.ends_with("| Apple { sub: Text } | Corp { domain: Text }"),
        "{t}"
    );
    assert_eq!(
        "set OAuth of identities = Google | Apple | Corp",
        decl(&s, "set", "identities.OAuth")
    );
    assert!(flat(&decl(&s, "fn", "display_name")).ends_with("| Corp c -> c.domain"));
    assert!(
        decl(&s, "constraint", "login_present").contains("| Corp c -> scalar_len(c.domain) == 0")
    );
    assert_eq!(Stage::Declared, stage(&s, "identities", "Corp"));
}

#[test]
fn add_constructor_places_it_before_or_after_another() {
    let s = applies(
        &running(),
        "module auth
add constructor Identity.Corp { domain: Text } before Google
  arm display_name: Corp c -> c.domain
  arm login_present: Corp c -> scalar_len(c.domain) == 0",
    );
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(
        t.contains("| Corp { domain: Text } | Google { gid: Text }"),
        "{t}"
    );
    let s = applies(
        &running(),
        "module auth
add constructor Identity.Corp after EmailPass
  arm display_name: Corp c -> \"corp\"
  arm login_present: Corp c -> false",
    );
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(
        t.contains("EmailPass { email: Text, hash: Bytes } | Corp | Google"),
        "{t}"
    );
    refuses(
        &running(),
        "module auth\nadd constructor Identity.Corp before Nope",
        "`Nope` is not a constructor of `Identity`",
    );
}

#[test]
fn a_match_without_the_new_arm_fails_the_recheck() {
    refuses(
        &running(),
        "module auth\nadd constructor Identity.Corp { domain: Text }",
        "match does not cover `Corp",
    );
    let s = applies(
        &running(),
        "module auth
add constructor Identity.Corp { domain: Text }
  arm display_name: Corp c -> c.domain
drop constraint login_present
add constraint login_present = from i in identities.EmailPass where scalar_len(i.email) == 0 select i",
    );
    let _ = s;
}

#[test]
fn add_constructor_refuses_what_it_cannot_add() {
    refuses(
        &running(),
        "module auth\nadd constructor Identity.Google { x: Text }",
        "`Google` is already a constructor of `Identity`",
    );
    refuses(
        &running(),
        "module auth\nadd constructor Identity.OAuth",
        "`OAuth` is already a named set of `identities`",
    );
    refuses(
        &running(),
        "module auth\nadd constructor Identity.Corp joins users.Nope",
        "`users` is not a relation variable of `Identity`",
    );
    refuses(
        &running(),
        "module auth\nadd constructor Identity.Corp joins identities.Nope",
        "declares no named set `Nope`",
    );
    refuses(
        &running(),
        "module auth\nadd constructor Identity.Corp arm nosuch: Corp c -> 1",
        "declares no schema function, constraint or computed field `nosuch`",
    );
}

#[test]
fn an_arm_targets_exactly_one_match() {
    let s = applies(
        &running(),
        "module auth
redefine fn display_name (i: Identity) -> Text = match i
  | EmailPass e -> e.email
  | Google g    -> g.gid
  | Apple a     -> a.sub
define fn both (i: Identity, j: Identity) -> Text =
  (match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub)
  ++ (match j | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub)",
    );
    refuses(
        &s,
        "module auth
add constructor Identity.Corp
  arm display_name: Corp c -> \"c\"
  arm login_present: Corp c -> false
  arm both: Corp c -> \"c\"",
        "`both` has more than one match over the constructors this arm is for",
    );
    let s = applies(
        &running(),
        "module auth\ndefine fn mail (e: Text) -> Text = e",
    );
    refuses(
        &s,
        "module auth
add constructor Identity.Corp
  arm mail: Corp c -> \"c\"",
        "`mail` has no match over the constructors this arm is for",
    );
}

#[test]
fn an_arm_for_a_computed_field_goes_to_its_match() {
    let s = source(
        "module auth
type Shape { id: Key shapes } = Circle { r: F64 } | Rect { w: F64, h: F64 }
relation shapes : Shape
  key id generate sequence
  computed area: F64 = match self
    | Circle c -> 3.14159 * c.r * c.r
    | Rect r   -> r.w * r.h",
    );
    let s = applies(
        &s,
        "module auth
add constructor Shape.Square { side: F64 }
  arm shapes.area: Square q -> q.side * q.side",
    );
    assert!(decl(&s, "relation", "shapes").contains("| Square q -> q.side * q.side"));
}

#[test]
fn an_operator_on_a_type_reaches_every_relation_of_it() {
    let s = applies(
        &with_badges(),
        "module auth
add constructor Badge.Emoji { glyph: Text }
  joins post_badges.Simple",
    );
    assert_eq!(Stage::Declared, stage(&s, "post_badges", "Emoji"));
    assert_eq!(Stage::Declared, stage(&s, "user_badges", "Emoji"));
    assert_eq!(
        "set Simple of post_badges = Plain | Emoji",
        decl(&s, "set", "post_badges.Simple")
    );
    let s = applies(&s, "module auth\nenable post_badges.Emoji");
    assert_eq!(Stage::Enabled, stage(&s, "post_badges", "Emoji"));
    assert_eq!(Stage::Declared, stage(&s, "user_badges", "Emoji"));
}

#[test]
fn write_states_move_by_enable_disable_and_deprecate() {
    let s = applies(
        &running(),
        "module auth
add constructor Identity.Corp
  arm display_name: Corp c -> \"c\"
  arm login_present: Corp c -> false",
    );
    refuses(
        &s,
        "module auth\ndeprecate identities.Corp",
        "`deprecate` applies only to an enabled constructor; `identities.Corp` is declared",
    );
    refuses(
        &s,
        "module auth\ndisable identities.Corp",
        "`identities.Corp` is declared",
    );
    let s = applies(&s, "module auth\nenable identities.Corp");
    assert_eq!(Stage::Enabled, stage(&s, "identities", "Corp"));
    refuses(
        &s,
        "module auth\nenable identities.Corp",
        "`identities.Corp` is already enabled",
    );
    let d = applies(&s, "module auth\ndisable identities.Corp");
    assert_eq!(Stage::Declared, stage(&d, "identities", "Corp"));
    let s = applies(&s, "module auth\ndeprecate identities.Corp");
    assert_eq!(Stage::Deprecated, stage(&s, "identities", "Corp"));
    refuses(
        &s,
        "module auth\ndisable identities.Corp",
        "`identities.Corp` is deprecated",
    );
    let s = applies(&s, "module auth\nenable identities.Corp");
    assert_eq!(Stage::Enabled, stage(&s, "identities", "Corp"));
    refuses(
        &s,
        "module auth\nenable identities.OAuth",
        "names one constructor",
    );
}

#[test]
fn value_level_constructors_have_states_per_type() {
    let s = applies(&running(), "module auth\nadd type Provider = Google | Okta");
    let key = (
        "auth".to_string(),
        "Provider".to_string(),
        "Okta".to_string(),
    );
    assert_eq!(Stage::Enabled, s.value_states[&key].stage);
    let s = applies(&s, "module auth\nadd constructor Provider.Saml");
    let saml = (
        "auth".to_string(),
        "Provider".to_string(),
        "Saml".to_string(),
    );
    assert_eq!(Stage::Declared, s.value_states[&saml].stage);
    let s = applies(&s, "module auth\nenable Provider.Saml");
    assert_eq!(Stage::Enabled, s.value_states[&saml].stage);
    refuses(
        &s,
        "module auth\nenable Provider.Nope",
        "`Nope` is not a constructor of `Provider`",
    );
}

#[test]
fn remove_needs_a_constructor_deprecated_by_an_earlier_migration() {
    refuses(
        &running(),
        "module auth\nremove constructor Identity.Apple",
        "`remove constructor Identity.Apple` is refused: `Apple` is not deprecated in `identities` by an earlier migration",
    );
    refuses(
        &running(),
        "module auth
deprecate identities.Apple
remove constructor Identity.Apple",
        "is not deprecated in `identities` by an earlier migration",
    );
    let s = applies(&running(), "module auth\ndeprecate identities.Apple");
    let s = applies(
        &s,
        "module auth
remove constructor Identity.Apple
redefine fn display_name (i: Identity) -> Text = match i
  | EmailPass e -> e.email
  | Google g    -> g.gid",
    );
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(t.ends_with("| Google { gid: Text }"), "{t}");
    assert_eq!(
        "set OAuth of identities = Google",
        decl(&s, "set", "identities.OAuth")
    );
    let r = decl(&s, "relation", "identities");
    assert!(!r.contains("Apple"), "{r}");
    assert!(!decl(&s, "constraint", "login_present").contains("Apple"));
    assert!(
        !s.states
            .contains_key(&("auth".into(), "identities".into(), "Apple".into()))
    );
}

#[test]
fn remove_leaves_functions_that_name_it_to_the_recheck() {
    let s = applies(&running(), "module auth\ndeprecate identities.Apple");
    refuses(
        &s,
        "module auth\nremove constructor Identity.Apple",
        "Apple",
    );
}

#[test]
fn remove_is_refused_while_a_reference_or_a_set_needs_the_constructor() {
    let s = applies(
        &running(),
        "module auth
add type Pin = { id: Key pins, login: Key identities.(Google | Apple) }
add relation pins : Pin key id generate sequence
deprecate identities.Apple",
    );
    refuses(
        &s,
        "module auth\nremove constructor Identity.Apple",
        "field `Pin.login` references a constructor set that holds it",
    );
    let s = applies(
        &running(),
        "module auth\nadd set Only of identities = Apple\ndeprecate identities.Apple",
    );
    refuses(
        &s,
        "module auth\nremove constructor Identity.Apple",
        "the named set `Only` of `identities` would become empty",
    );
    let s = applies(
        &running(),
        "module auth\nadd check sub_set on identities.Apple (scalar_len(sub) > 0)\ndeprecate identities.Apple",
    );
    refuses(
        &s,
        "module auth\nremove constructor Identity.Apple",
        "check `sub_set` requires rows of `Apple`",
    );
}

#[test]
fn split_replaces_a_constructor_by_its_routing() {
    let s = source(
        "module auth
type User = { id: Key users, name: Text }
type Provider = Google | Apple
type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes } | OAuth { provider: Provider, sub: Text }
relation users : User key id generate sequence
relation identities : Identity key id generate uuidv7 unique (provider, sub) in OAuth
fn display_name (i: Identity) -> Text = match i
  | EmailPass e -> e.email
  | OAuth o -> o.sub",
    );
    let s = applies(
        &s,
        "module auth
split constructor Identity.OAuth into Google { gid: Text } | Apple { sub: Text }
  by match
    | OAuth { provider: Google, sub } -> Google { gid: sub }
    | OAuth { provider: Apple, sub }  -> Apple { sub: sub }
  unique (gid) in Google
  unique (sub) in Apple
  arm display_name: Google g -> g.gid
  arm display_name: Apple a -> a.sub",
    );
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(
        t.ends_with("| Google { gid: Text } | Apple { sub: Text }"),
        "{t}"
    );
    let r = flat(&decl(&s, "relation", "identities"));
    assert!(r.contains("unique (gid) in Google"), "{r}");
    assert!(r.contains("unique (sub) in Apple"), "{r}");
    assert!(!r.contains("OAuth"), "{r}");
    let f = flat(&decl(&s, "fn", "display_name"));
    assert!(f.contains("| Google g -> g.gid | Apple a -> a.sub"), "{f}");
    assert!(!f.contains("OAuth"), "{f}");
    assert_eq!(Stage::Enabled, stage(&s, "identities", "Google"));
}

const PLACES: &str = "module auth
type Scale = Realm | Town | Spot
type Place { id: Key places, name: Text }
  = Location { parent: Option (Key places.Location), scale: Scale }
  | Faction { agenda: Text }
relation places : Place key id generate sequence
type Npc = { id: Key npcs, location: Option (Key places.Location) on delete clear }
relation npcs : Npc key id generate sequence
fn kind (p: Place) -> Text = match p
  | Location _ -> \"Location\"
  | Faction _ -> \"Faction\"
fn place_name (l: Place.Location) -> Text = l.name
constraint parent_elsewhere =
  from l in places.Location, p in places.Location
  where l.parent == Some { value: key p } and key p == key l
  select l";

const SPLIT_PLACES: &str = "module auth
split constructor Place.Location
  into Region { parent: Option (Key places.(Region | Settlement | Site)) }
     | Settlement { parent: Option (Key places.(Region | Settlement | Site)) }
     | Site { parent: Option (Key places.(Region | Settlement | Site)) }
  by match
    | Location { parent, scale: Realm } -> Region { parent: parent }
    | Location { parent, scale: Town } -> Settlement { parent: parent }
    | Location { parent, scale: Spot } -> Site { parent: parent }
  arm kind: Region _ -> \"Region\"
  arm kind: Settlement _ -> \"Settlement\"
  arm kind: Site _ -> \"Site\"";

fn assert_places_split(s: &Source) {
    let products = "Option (Key places.(Region | Settlement | Site))";
    assert_eq!(
        flat(&format!(
            "type Place {{ id: Key places, name: Text }} = Region {{ parent: {products} }} \
             | Settlement {{ parent: {products} }} | Site {{ parent: {products} }} \
             | Faction {{ agenda: Text }}"
        )),
        flat(&decl(s, "type", "Place"))
    );
    assert!(
        flat(&decl(s, "type", "Npc")).contains(&format!("location: {products} on delete clear")),
        "{}",
        decl(s, "type", "Npc")
    );
    assert_eq!(
        "fn place_name (l: Place.(Region | Settlement | Site)) -> Text = l.name",
        decl(s, "fn", "place_name")
    );
    let c = flat(&decl(s, "constraint", "parent_elsewhere"));
    assert!(
        c.contains("from l in places.(Region | Settlement | Site), p in places.(Region | Settlement | Site)"),
        "{c}"
    );
}

#[test]
fn a_split_carries_a_reference_to_itself_at_its_retyped_type() {
    let s = applies(&source(PLACES), SPLIT_PLACES);
    assert_places_split(&s);
}

#[test]
fn a_split_product_naming_the_split_constructor_names_the_products() {
    let s = applies(
        &source(PLACES),
        &SPLIT_PLACES.replace("places.(Region | Settlement | Site)", "places.Location"),
    );
    assert_places_split(&s);
}

#[test]
fn a_split_product_may_name_one_product() {
    let without_constraint = PLACES.split("\nconstraint").next().expect("a schema");
    let s = applies(
        &source(without_constraint),
        "module auth
split constructor Place.Location
  into Region { parent: Option (Key places.Region) }
     | Settlement { parent: Option (Key places.Region) }
     | Site { parent: Option (Key places.Settlement) }
  by match
    | Location { scale: Realm } -> Region { parent: None }
    | Location { scale: Town } -> Settlement { parent: None }
    | Location { scale: Spot } -> Site { parent: None }
  arm kind: Region _ -> \"Region\"
  arm kind: Settlement _ -> \"Settlement\"
  arm kind: Site _ -> \"Site\"",
    );
    let t = flat(&decl(&s, "type", "Place"));
    assert!(
        t.contains("Site { parent: Option (Key places.Settlement) }"),
        "{t}"
    );
}

/// `PLACES` with its scale named after the products, which a merge back needs as its
/// discriminator, and `SPLIT_PLACES` over it.
fn places_by_product() -> (Source, String) {
    let scales = [
        ("Realm", "Region"),
        ("Town", "Settlement"),
        ("Spot", "Site"),
    ];
    let (mut schema, mut split) = (PLACES.to_string(), SPLIT_PLACES.to_string());
    for (old, new) in scales {
        schema = schema.replace(old, new);
        split = split.replace(&format!("scale: {old}"), &format!("scale: {new}"));
    }
    (source(&schema), split)
}

#[test]
fn revert_of_a_split_of_a_constructor_referencing_itself_merges_back() {
    let (base, split) = places_by_product();
    let s = applies(&base, &split);
    let s = applies(&s, "module auth\nrevert 0001");
    let t = flat(&decl(&s, "type", "Place"));
    assert!(
        t.contains("Location { parent: Option (Key places.Location), scale: Scale }"),
        "{t}"
    );
    assert!(
        flat(&decl(&s, "type", "Npc")).contains("Option (Key places.Location)"),
        "{}",
        decl(&s, "type", "Npc")
    );
    assert_eq!(
        "fn place_name (l: Place.Location) -> Text = l.name",
        decl(&s, "fn", "place_name")
    );
    assert!(
        flat(&decl(&s, "constraint", "parent_elsewhere"))
            .contains("from l in places.Location, p in places.Location"),
        "{}",
        decl(&s, "constraint", "parent_elsewhere")
    );
}

#[test]
fn a_merge_of_constructors_referencing_themselves_names_the_merged_one() {
    let (base, split) = places_by_product();
    let s = applies(&base, &split);
    let s = applies(
        &s,
        "module auth
merge constructor Place.(Region | Settlement | Site)
  into Location { parent: Option (Key places.Location), scale: Scale }
  by match
    | Region { parent } -> Location { parent: parent, scale: Region }
    | Settlement { parent } -> Location { parent: parent, scale: Settlement }
    | Site { parent } -> Location { parent: parent, scale: Site }
  arm kind: Location _ -> \"Location\"",
    );
    let t = flat(&decl(&s, "type", "Place"));
    assert!(
        t.contains("= Location { parent: Option (Key places.Location), scale: Scale } | Faction"),
        "{t}"
    );
}

#[test]
fn an_arm_goes_inside_the_parentheses_around_its_match() {
    let base = source(&format!(
        "{RUNNING}constraint login_long =
  from i in identities
  where 3 > (match i
    | EmailPass e -> scalar_len(e.email)
    | Google g -> scalar_len(g.gid)
    | Apple a -> scalar_len(a.sub))
  select i
"
    ));
    let s = applies(
        &base,
        "module auth
add constructor Identity.Corp { domain: Text }
  arm display_name: Corp c -> c.domain
  arm login_present: Corp c -> scalar_len(c.domain) == 0
  arm login_long: Corp c -> scalar_len(c.domain)",
    );
    assert!(
        flat(&decl(&s, "constraint", "login_long"))
            .contains("| Apple a -> scalar_len(a.sub) | Corp c -> scalar_len(c.domain)) select i"),
        "{}",
        decl(&s, "constraint", "login_long")
    );
}

#[test]
fn a_split_arm_goes_inside_the_parentheses_around_its_match() {
    let base = source(&format!(
        "{PLACES}
constraint long_names =
  from p in places
  where 3 > (match p
    | Location l -> scalar_len(l.name)
    | Faction f -> scalar_len(f.agenda))
  select p
"
    ));
    let s = applies(
        &base,
        &format!(
            "{SPLIT_PLACES}
  arm long_names: Region r -> scalar_len(r.name)
  arm long_names: Settlement t -> scalar_len(t.name)
  arm long_names: Site t -> scalar_len(t.name)"
        ),
    );
    let c = flat(&decl(&s, "constraint", "long_names"));
    assert!(
        c.ends_with("| Site t -> scalar_len(t.name)) select p"),
        "{c}"
    );
}

#[test]
fn a_split_routing_is_checked_exhaustive_and_typed() {
    let s = source(
        "module auth
type Provider = Google | Apple
type Identity { id: Key identities } = Mail { email: Text } | OAuth { provider: Provider, sub: Text }
relation identities : Identity key id generate uuidv7",
    );
    refuses(
        &s,
        "module auth
split constructor Identity.OAuth into Google { gid: Text } | Apple { sub: Text }
  by match
    | OAuth { provider: Google, sub } -> Google { gid: sub }",
        "match does not cover",
    );
    refuses(
        &s,
        "module auth
split constructor Identity.OAuth into Google { gid: Text } | Apple { sub: Text }
  by match
    | OAuth { provider: Google, sub } -> Google { gid: sub }
    | OAuth { provider: Apple, sub }  -> Mail { email: sub }",
        "Mail",
    );
    refuses(
        &s,
        "module auth
split constructor Identity.OAuth into Mail { x: Text } | Apple { sub: Text }
  by match
    | OAuth o -> Apple { sub: o.sub }",
        "`Mail` is already a constructor of `Identity`",
    );
}

#[test]
fn a_split_needs_replacements_for_an_index_on_a_lost_field() {
    let s = source(
        "module auth
type Provider = Google | Apple
type Identity { id: Key identities } = Mail { email: Text } | OAuth { provider: Provider, sub: Text }
relation identities : Identity key id generate uuidv7 unique (provider, sub) in OAuth",
    );
    refuses(
        &s,
        "module auth
split constructor Identity.OAuth into Google { gid: Text } | Apple { sub: Text }
  by match
    | OAuth { provider: Google, sub } -> Google { gid: sub }
    | OAuth { provider: Apple, sub }  -> Apple { sub: sub }",
        "names `provider`, which no new constructor keeps; state its replacements after the routing",
    );
    let s = source(
        "module auth
type Identity { id: Key identities } = Mail { email: Text } | OAuth { kind: Text, sub: Text }
relation identities : Identity key id generate uuidv7 index (sub) in OAuth",
    );
    let s = applies(
        &s,
        "module auth
split constructor Identity.OAuth into Google { gid: Text } | Apple { sub: Text }
  by match
    | OAuth { kind: \"g\", sub } -> Google { gid: sub }
    | OAuth o -> Apple { sub: o.sub }",
    );
    let r = flat(&decl(&s, "relation", "identities"));
    assert!(r.contains("index (gid) in Google"), "{r}");
    assert!(r.contains("index (sub) in Apple"), "{r}");
}

#[test]
fn merge_replaces_constructors_by_one_with_a_discriminator() {
    let s = applies(
        &running(),
        "module auth
add type Provider = Google | Apple
merge constructor Identity.(Google | Apple) into Social
  { provider: Provider, sub: Text }
  by match
    | Google g -> Social { provider: Google, sub: g.gid }
    | Apple a  -> Social { provider: Apple, sub: a.sub }
  arm display_name: Social s -> s.sub
  arm login_present: Social s -> scalar_len(s.sub) == 0",
    );
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(
        t.ends_with("| Social { provider: Provider, sub: Text }"),
        "{t}"
    );
    assert!(!t.contains("Google"), "{t}");
    let r = flat(&decl(&s, "relation", "identities"));
    assert!(r.contains("unique (provider, sub) in Social"), "{r}");
    assert_eq!(1, r.matches("in Social").count(), "{r}");
    assert_eq!(
        "set OAuth of identities = Social",
        decl(&s, "set", "identities.OAuth")
    );
    let f = flat(&decl(&s, "fn", "display_name"));
    assert!(
        f.ends_with("| EmailPass e -> e.email | Social s -> s.sub"),
        "{f}"
    );
    assert_eq!(Stage::Enabled, stage(&s, "identities", "Social"));
    assert!(
        !s.states
            .contains_key(&("auth".into(), "identities".into(), "Google".into()))
    );
}

#[test]
fn a_merge_needs_one_discriminator() {
    refuses(
        &running(),
        "module auth
merge constructor Identity.(Google | Apple) into Social { sub: Text }
  by match
    | Google g -> Social { sub: g.gid }
    | Apple a  -> Social { sub: a.sub }",
        "the merged constructor `Social` has no discriminator",
    );
    refuses(
        &running(),
        "module auth
add type Provider = Google | Apple
add type Again = Google | Apple
merge constructor Identity.(Google | Apple) into Social { provider: Provider, again: Again, sub: Text }",
        "has more than one discriminator field",
    );
    refuses(
        &running(),
        "module auth
add type Provider = Google | Apple
merge constructor Identity.(Google | Apple) into Social { provider: Provider, sub: Text }
  by match
    | Google g -> Social { provider: Apple, sub: g.gid }
    | Apple a  -> Social { provider: Apple, sub: a.sub }",
        "the routing gives the discriminator `provider` as each row's own constructor name",
    );
}

#[test]
fn a_merge_without_routing_maps_fields_by_name() {
    let s = source(
        "module auth
type Identity { id: Key identities } = Mail { email: Text } | Google { sub: Text } | Apple { sub: Text }
relation identities : Identity key id generate uuidv7
type Provider = Google | Apple",
    );
    let s2 = applies(
        &s,
        "module auth\nmerge constructor Identity.(Google | Apple) into Social { provider: Provider, sub: Text }",
    );
    assert!(
        flat(&decl(&s2, "type", "Identity")).contains("Social { provider: Provider, sub: Text }")
    );
    refuses(
        &s,
        "module auth\nmerge constructor Identity.(Google | Apple) into Social { provider: Provider, other: Text }",
        "field `Google.sub` has no counterpart in `Social`",
    );
}

#[test]
fn a_merge_refuses_a_partial_set_and_a_deprecated_source() {
    let s = applies(
        &running(),
        "module auth\nadd set Gg of identities = Google | EmailPass",
    );
    refuses(
        &s,
        "module auth
add type Provider = Google | Apple
merge constructor Identity.(Google | Apple) into Social { provider: Provider, sub: Text }
  by match
    | Google g -> Social { provider: Google, sub: g.gid }
    | Apple a  -> Social { provider: Apple, sub: a.sub }",
        "the named set `Gg` of `identities` holds some but not all of its constructors",
    );
    let s = applies(&running(), "module auth\ndeprecate identities.Apple");
    refuses(
        &s,
        "module auth
add type Provider = Google | Apple
merge constructor Identity.(Google | Apple) into Social { provider: Provider, sub: Text }
  by match
    | Google g -> Social { provider: Google, sub: g.gid }
    | Apple a  -> Social { provider: Apple, sub: a.sub }",
        "merging or nesting a deprecated constructor is refused: `identities.Apple` is deprecated",
    );
}

#[test]
fn a_merge_product_is_declared_when_a_source_is() {
    let s = applies(
        &running(),
        "module auth
add constructor Identity.Corp { sub: Text }
  joins identities.OAuth
  arm display_name: Corp c -> c.sub
  arm login_present: Corp c -> false",
    );
    let s = applies(
        &s,
        "module auth
add type Kind = Apple | Corp
merge constructor Identity.(Apple | Corp) into Other { kind: Kind, sub: Text }
  arm display_name: Other o -> o.sub
  arm login_present: Other o -> false",
    );
    assert_eq!(Stage::Declared, stage(&s, "identities", "Other"));
}

#[test]
fn nest_moves_constructors_into_a_field_and_rescopes_by_path() {
    let s = applies(
        &running(),
        "module auth
add type OAuthLogin = Google { gid: Text } | Apple { sub: Text }
nest constructor Identity.(Google | Apple) into Social { login: OAuthLogin }
  arm display_name: Social s -> match s.login | Google g -> g.gid | Apple a -> a.sub
  arm login_present: Social s ->
    match s.login | Google g -> scalar_len(g.gid) == 0 | Apple a -> scalar_len(a.sub) == 0",
    );
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(t.ends_with("| Social { login: OAuthLogin }"), "{t}");
    let r = flat(&decl(&s, "relation", "identities"));
    assert!(r.contains("unique (login.Google.gid) in Social"), "{r}");
    assert!(r.contains("unique (login.Apple.sub) in Social"), "{r}");
    refuses(
        &running(),
        "module auth
add type Other = Google { x: Text } | Apple { sub: Text }
nest constructor Identity.(Google | Apple) into Social { login: Other }",
        "holds an existing sum whose constructors have the names and fields of Google, Apple",
    );
}

#[test]
fn unnest_lifts_the_constructors_of_a_field() {
    let s = source(
        "module auth
type OAuthLogin = Google { gid: Text } | Apple { sub: Text }
type Identity { id: Key identities } = Mail { email: Text } | Social { login: OAuthLogin }
relation identities : Identity key id generate uuidv7 unique (login.Google.gid) in Social
fn name (i: Identity) -> Text = match i | Mail m -> m.email | Social s -> match s.login | Google g -> g.gid | Apple a -> a.sub",
    );
    let s = applies(
        &s,
        "module auth
unnest constructor Identity.Social
  arm name: Google g -> g.gid
  arm name: Apple a -> a.sub",
    );
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(
        t.ends_with("| Google { gid: Text } | Apple { sub: Text }"),
        "{t}"
    );
    let r = flat(&decl(&s, "relation", "identities"));
    assert!(r.contains("unique (gid) in Google"), "{r}");
    assert!(flat(&decl(&s, "fn", "name")).ends_with("| Google g -> g.gid | Apple a -> a.sub"));
}

#[test]
fn fields_are_added_moved_and_dropped() {
    let s = applies(
        &running(),
        "module auth\nadd field Identity.EmailPass.verified: Bool fill false",
    );
    assert!(
        flat(&decl(&s, "type", "Identity"))
            .contains("EmailPass { email: Text, hash: Bytes, verified: Bool }")
    );
    let s = applies(
        &s,
        "module auth
move field Identity.EmailPass.verified to Identity fill match
  | Google g -> true
  | Apple a  -> true",
    );
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(
        t.starts_with("type Identity { id: Key identities, user: Key users, verified: Bool }"),
        "{t}"
    );
    assert!(t.contains("EmailPass { email: Text, hash: Bytes }"), "{t}");
    let s = applies(
        &s,
        "module auth\nmove field Identity.verified to Identity.(EmailPass | Google)",
    );
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(
        t.contains("EmailPass { email: Text, hash: Bytes, verified: Bool }"),
        "{t}"
    );
    assert!(t.contains("Google { gid: Text, verified: Bool }"), "{t}");
    assert!(t.contains("Apple { sub: Text }"), "{t}");
    let s = applies(&s, "module auth\ndrop field Identity.EmailPass.hash");
    assert!(
        flat(&decl(&s, "type", "Identity")).contains("EmailPass { email: Text, verified: Bool }")
    );
}

#[test]
fn add_field_states_a_delete_action_and_alter_field_changes_it() {
    let s = applies(
        &running(),
        "module auth
add field Identity.EmailPass.invited_by: Option (Key users) on delete clear fill None",
    );
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(
        t.contains(
            "EmailPass { email: Text, hash: Bytes, invited_by: Option (Key users) on delete clear }"
        ),
        "{t}"
    );
    let s = applies(
        &s,
        "module auth\nalter field Identity.EmailPass.invited_by on delete restrict",
    );
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(
        t.contains("invited_by: Option (Key users) on delete restrict }"),
        "{t}"
    );
    let s = applies(
        &s,
        "module auth\nalter field Identity.user on delete cascade",
    );
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(t.contains("user: Key users on delete cascade }"), "{t}");
    let s = applies(&s, "module auth\nrevert 0003");
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(t.contains("user: Key users on delete restrict }"), "{t}");
    let s = applies(&s, "module auth\nrevert 0002");
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(
        t.contains("invited_by: Option (Key users) on delete clear }"),
        "{t}"
    );
}

#[test]
fn alter_field_is_refused_on_a_field_that_is_not_a_reference() {
    refuses(
        &running(),
        "module auth\nalter field Identity.EmailPass.email on delete cascade",
        "`on delete` is allowed only on a `Key` field or an `Option` of one",
    );
    refuses(
        &running(),
        "module auth\nalter field Identity.user on delete clear",
        "`on delete clear` is allowed only on an `Option (Key ...)` field",
    );
    refuses(
        &running(),
        "module auth\nalter field Identity.EmailPass.nothing on delete cascade",
        "has no field `nothing`",
    );
}

#[test]
fn revert_of_a_dropped_reference_field_restores_its_delete_action() {
    let s = applies(
        &running(),
        "module auth
add field Identity.EmailPass.invited_by: Option (Key users) on delete clear fill None",
    );
    let s = applies(&s, "module auth\ndrop field Identity.EmailPass.invited_by");
    let s = applies(
        &s,
        "module auth\nrevert 0002\n  fill Identity.EmailPass.invited_by = None",
    );
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(
        t.contains("invited_by: Option (Key users) on delete clear }"),
        "{t}"
    );
}

/// The source of module `auth` holding users and npcs, whose `owner` field is written `owner`.
fn with_owner(owner: &str) -> Source {
    source(&format!(
        "
module auth
type User = {{ id: Key users, name: Text }}
relation users : User key id generate sequence
type Npc = {{ id: Key npcs, {owner}
  , name: Text }}
relation npcs : Npc key id generate sequence
"
    ))
}

#[test]
fn revert_of_a_dropped_field_ignores_a_comment_after_its_delete_action() {
    let s = with_owner("owner: Option (Key users) on delete clear -- keep the npc\n");
    let s = applies(&s, "module auth\ndrop field Npc.owner");
    let s = applies(&s, "module auth\nrevert 0001\n  fill Npc.owner = None");
    let t = flat(&decl(&s, "type", "Npc"));
    assert!(
        t.contains("owner: Option (Key users) on delete clear"),
        "{t}"
    );
}

#[test]
fn alter_field_keeps_a_comment_after_the_delete_action() {
    let s = with_owner("owner: Option (Key users) on delete clear -- keep the npc\n");
    let s = applies(&s, "module auth\nalter field Npc.owner on delete restrict");
    let t = decl(&s, "type", "Npc");
    assert!(
        t.contains("owner: Option (Key users) on delete restrict -- keep the npc\n"),
        "{t}"
    );
}

#[test]
fn alter_field_takes_a_comment_holding_a_comma_as_a_comment() {
    let s = with_owner("owner: Option (Key users) on delete clear -- keep, really\n");
    let s = applies(&s, "module auth\nalter field Npc.owner on delete restrict");
    let t = decl(&s, "type", "Npc");
    assert!(
        t.contains("owner: Option (Key users) on delete restrict -- keep, really\n"),
        "{t}"
    );
}

#[test]
fn revert_of_alter_field_restores_an_action_after_a_comment() {
    let s = with_owner("owner: Option (Key users) -- the player\n    on delete clear\n");
    let s = applies(&s, "module auth\nalter field Npc.owner on delete restrict");
    let t = flat(&decl(&s, "type", "Npc"));
    assert!(t.contains("-- the player on delete restrict"), "{t}");
    let s = applies(&s, "module auth\nrevert 0001");
    let t = flat(&decl(&s, "type", "Npc"));
    assert!(t.contains("-- the player on delete clear"), "{t}");
}

#[test]
fn alter_field_adds_an_action_to_a_field_without_one() {
    let s = with_owner("owner: Option (Key users) -- the player\n");
    let s = applies(&s, "module auth\nalter field Npc.owner on delete clear");
    let t = decl(&s, "type", "Npc");
    assert!(
        t.contains("owner: Option (Key users) on delete clear -- the player\n"),
        "{t}"
    );
}

#[test]
fn a_fill_is_typed_and_reads_only_its_row() {
    refuses(
        &running(),
        "module auth\nadd field Identity.EmailPass.verified: Bool fill 3",
        "Bool",
    );
    refuses(
        &running(),
        "module auth\nadd field Identity.EmailPass.n: I64 fill count(u for u in users)",
        "a fill or a routing reads only its row",
    );
    applies(
        &running(),
        "module auth\nadd field Identity.EmailPass.n: I64 fill match | EmailPass e -> scalar_len(e.email)",
    );
    refuses(
        &running(),
        "module auth
move field Identity.EmailPass.email to Identity fill match
  | Google g -> g.gid",
        "match does not cover",
    );
    refuses(
        &running(),
        "module auth\nmove field Identity.EmailPass.email to Identity fill \"\"",
        "is a `match` whose arms are the constructors that lack the field",
    );
}

#[test]
fn field_operators_refuse_what_is_not_there() {
    refuses(
        &running(),
        "module auth\ndrop field Identity.EmailPass.nope",
        "`Identity.EmailPass` has no field `nope`",
    );
    refuses(
        &running(),
        "module auth\ndrop field Identity.Nope.x",
        "`Nope` is not a constructor of `Identity`",
    );
    refuses(
        &running(),
        "module auth\nadd field Identity.Google.gid: Text fill \"\"",
        "`Identity.Google` already has a field `gid`",
    );
    refuses(
        &running(),
        "module auth\nadd field Identity.gid: Text fill \"\"",
        "`Identity` already has a field `gid`",
    );
    refuses(
        &running(),
        "module auth\nmove field Identity.EmailPass.email to User fill match | Google g -> \"\" | Apple a -> \"\"",
        "a field moves within its own type",
    );
}

#[test]
fn a_record_field_is_its_type_field() {
    let s = applies(
        &running(),
        "module auth\nadd field User.email: Text fill \"\"",
    );
    assert_eq!(
        "type User = { id: Key users, name: Text, email: Text }",
        decl(&s, "type", "User")
    );
    let s = applies(&s, "module auth\nrename field User.email to mail");
    assert_eq!(
        "type User = { id: Key users, name: Text, mail: Text }",
        decl(&s, "type", "User")
    );
}

#[test]
fn uniques_and_indexes_are_added_and_dropped() {
    let s = applies(
        &running(),
        "module auth
add unique (user) in identities.Google
add index (user) in identities
add interval index (user) in identities.(Google | Apple)
add unique (lower(email)) in identities.EmailPass",
    );
    let r = decl(&s, "relation", "identities");
    assert!(r.contains("unique (user) in Google"), "{r}");
    assert!(r.contains("\n  index (user)\n"), "{r}");
    assert!(
        r.contains("interval index (user) in (Google | Apple)"),
        "{r}"
    );
    assert!(r.contains("unique (lower(email)) in EmailPass"), "{r}");
    let s = applies(
        &s,
        "module auth\ndrop unique (user) in identities.Google\ndrop index (user) in identities",
    );
    let r = decl(&s, "relation", "identities");
    assert!(!r.contains("unique (user)"), "{r}");
    assert!(!r.contains("\n  index (user)"), "{r}");
    refuses(
        &running(),
        "module auth\ndrop unique (user) in identities.Google",
        "`identities` has no unique (user) in Google",
    );
    refuses(
        &running(),
        "module auth\nadd index (nope) in identities",
        "nope",
    );
}

#[test]
fn defaults_and_computed_fields_are_added_and_dropped() {
    let s = applies(
        &running(),
        "module auth
add field Identity.EmailPass.verified: Bool fill false
add default identities.EmailPass.verified = false",
    );
    assert!(decl(&s, "relation", "identities").contains("default EmailPass.verified = false"));
    let s = applies(
        &s,
        "module auth\ndrop default identities.EmailPass.verified",
    );
    assert!(!decl(&s, "relation", "identities").contains("default"));
    refuses(
        &s,
        "module auth\ndrop default identities.EmailPass.verified",
        "`identities` has no default for `verified`",
    );
    let s = applies(
        &running(),
        "module auth\nadd computed users.shout: Text = upper(self.name)",
    );
    assert!(decl(&s, "relation", "users").contains("computed shout: Text = upper(self.name)"));
    let s = applies(
        &s,
        "module auth\nredefine computed users.shout: Text = lower(self.name)",
    );
    assert!(decl(&s, "relation", "users").contains("computed shout: Text = lower(self.name)"));
    let s = applies(&s, "module auth\ndrop computed users.shout");
    assert!(!decl(&s, "relation", "users").contains("computed"));
    refuses(
        &s,
        "module auth\ndrop computed users.shout",
        "`users` has no computed field `shout`",
    );
}

#[test]
fn rekey_carries_every_reference_and_drops_the_old_self_key() {
    let s = source(
        "module billing
type Price = { id: Key prices, sku: Text, during: Span Instant, amount: Decimal 2 }
relation prices : Price key id generate sequence
type Order { id: Key orders } = Line { price: Key prices, qty: I32 } | Note { text: Text }
relation orders : Order key id generate sequence",
    );
    let s2 = s.clone();
    let mig = |t: &str| format!("module billing\n{t}");
    let r = migrate(
        &s2,
        &mig(
            "rekey relation prices key (sku, during)\n  carry Order.Line.price\ndrop field Price.id",
        ),
    );
    let s3 = r.unwrap_or_else(|e| panic!("{e:?}"));
    let i = s3.find("relation", "billing", "prices").unwrap();
    assert_eq!(
        "relation prices : Price key (sku, during)",
        s3.decls[i].text
    );
    let errs = migrate(
        &s,
        &mig("rekey relation prices key (sku, during)\ndrop field Price.id"),
    )
    .unwrap_err();
    assert!(
        errs.iter()
            .any(|e| e.contains("field `Order.Line.price` references `prices` and is not carried")),
        "{errs:?}"
    );
    let errs = migrate(
        &s,
        &mig("rekey relation prices key (sku, during)\n  carry Order.Line.price"),
    )
    .unwrap_err();
    assert!(
        errs.iter().any(|e| e.contains("field `Price.id` of `prices`'s own type is typed `Key prices` and is outside the new key")),
        "{errs:?}"
    );
    let errs = migrate(&s, &mig("rekey relation prices key (sku, during)\n  carry Order.Line.price, Order.Note.text\ndrop field Price.id")).unwrap_err();
    assert!(
        errs.iter()
            .any(|e| e.contains("`Order.Note.text` is not a field that references `prices`")),
        "{errs:?}"
    );
}

#[test]
fn definitions_are_defined_redefined_and_dropped() {
    let s = applies(
        &running(),
        "module auth\ndefine fn shout (t: Text) -> Text = upper(t)",
    );
    assert_eq!(
        "fn shout (t: Text) -> Text = upper(t)",
        decl(&s, "fn", "shout")
    );
    let s = applies(
        &s,
        "module auth\nredefine fn shout (t: Text, n: I64) -> Text = upper(t)",
    );
    assert_eq!(
        "fn shout (t: Text, n: I64) -> Text = upper(t)",
        decl(&s, "fn", "shout")
    );
    let s = applies(&s, "module auth\ndrop fn shout");
    assert!(s.find("fn", "auth", "shout").is_none());
    refuses(
        &running(),
        "module auth\nredefine fn nosuch () -> I64 = 1",
        "declares no fn `nosuch`",
    );
    let s = applies(
        &running(),
        "module auth\ndefine fn shown (i: Identity) -> Text = display_name(i)",
    );
    refuses(&s, "module auth\ndrop fn display_name", "display_name");
    refuses(
        &running(),
        "module auth\ndefine fn display_name () -> I64 = 1",
        "already declares fn `display_name`",
    );
}

#[test]
fn constraints_checks_and_stored_views_come_and_go() {
    let s = applies(
        &running(),
        "module auth
add constraint named = from u in users where scalar_len(u.name) == 0 select u
add check gid_set on identities.Google (scalar_len(gid) > 0)
define fn names () -> Set Text = from u in users select u.name
store names",
    );
    assert!(s.find("constraint", "auth", "named").is_some());
    assert!(s.find("check", "auth", "gid_set").is_some());
    assert!(s.find("store", "auth", "names").is_some());
    let s = applies(
        &s,
        "module auth\ndrop constraint named\ndrop check gid_set\nunstore names",
    );
    assert!(s.find("constraint", "auth", "named").is_none());
    assert!(s.find("check", "auth", "gid_set").is_none());
    assert!(s.find("store", "auth", "names").is_none());
    refuses(
        &s,
        "module auth\nunstore names",
        "declares no store `names`",
    );
}

#[test]
fn a_script_is_a_write_at_the_schema_before_it() {
    applies(
        &running(),
        "module auth
add constructor Identity.Corp { domain: Text }
  arm display_name: Corp c -> c.domain
  arm login_present: Corp c -> scalar_len(c.domain) == 0
enable identities.Corp
script
  update i in identities.Google where ends_with(i.gid, \"@corp\")
  set Corp { i with domain: i.gid }",
    );
    refuses(
        &running(),
        "module auth\nscript\n  update i in identities.Google set Corp { i with domain: i.gid }",
        "Corp",
    );
}

#[test]
fn log_grants_name_schema_objects() {
    applies(
        &running(),
        "module auth\ngrant read identities.(EmailPass | Google) (user) to support",
    );
    applies(&running(), "module auth\ngrant call fn display_name to web");
    applies(
        &running(),
        "module auth\nrevoke read identities from support",
    );
    refuses(
        &running(),
        "module auth\ngrant read catalog::constructors to analyst",
        "a grant on a `catalog::` or `sys::` relation is deployment state",
    );
    refuses(
        &running(),
        "module auth\ngrant monitor to oncall",
        "the capability grant `monitor` is deployment state",
    );
    refuses(
        &running(),
        "module auth\ngrant call web-2.4 to web",
        "a bundle call grant is deployment state",
    );
    refuses(
        &running(),
        "module auth\ngrant read nosuch to support",
        "nosuch",
    );
}

#[test]
fn adopt_tzdb_changes_no_schema_object() {
    let s = applies(
        &running(),
        "module auth\nadopt tzdb \"2026b\" from \"tzdata-2026b.tzif\"",
    );
    assert_eq!(running().decls, s.decls);
}

#[test]
fn revert_undoes_a_migration_by_its_inverse() {
    let s = applies(&running(), "module auth\nadd index (user) in identities");
    let s = applies(&s, "module auth\nrevert 0001");
    assert_eq!(running().decls, s.decls);
    assert_eq!(2, s.version);
}

#[test]
fn revert_undoes_renames_last_first() {
    let s = applies(
        &running(),
        "module auth
rename constructor Identity.Google to Gmail
rename type Identity to Login
rename relation identities to logins",
    );
    let s = applies(&s, "module auth\nrevert 0001");
    assert_eq!(running().decls, s.decls);
}

#[test]
fn revert_of_an_added_constructor_removes_it_and_its_arms() {
    let s = applies(
        &running(),
        "module auth
add constructor Identity.Corp { domain: Text }
  arm display_name: Corp c -> c.domain
  arm login_present: Corp c -> scalar_len(c.domain) == 0",
    );
    let s = applies(&s, "module auth\nrevert 0001");
    assert_eq!(running().decls, s.decls);
    assert!(
        !s.states
            .contains_key(&("auth".into(), "identities".into(), "Corp".into()))
    );
}

#[test]
fn revert_restores_write_states() {
    let s = applies(&running(), "module auth\ndeprecate identities.Apple");
    let s = applies(&s, "module auth\nrevert 0001");
    assert_eq!(Stage::Enabled, stage(&s, "identities", "Apple"));
}

#[test]
fn revert_of_a_dropped_field_takes_its_fill() {
    let s = applies(
        &running(),
        "module auth\ndrop field Identity.EmailPass.hash",
    );
    refuses(&s, "module auth\nrevert 0001", "needs a fill");
    let s = applies(
        &s,
        "module auth\nrevert 0001\n  fill Identity.EmailPass.hash = bytes(\"\")",
    );
    assert!(flat(&decl(&s, "type", "Identity")).contains("EmailPass { email: Text, hash: Bytes }"));
}

#[test]
fn revert_is_refused_for_a_script_a_dependant_or_an_unknown_migration() {
    refuses(
        &running(),
        "module auth\nrevert 0001",
        "migration 0001 is not in the log",
    );
    let s = applies(
        &running(),
        "module auth\nadd index (user) in identities\nscript\n  delete u in users where u.name == \"\"",
    );
    refuses(&s, "module auth\nrevert 0001", "holds a `script`");
    let s = applies(
        &running(),
        "module auth
add constructor Identity.Corp { domain: Text }
  arm display_name: Corp c -> c.domain
  arm login_present: Corp c -> scalar_len(c.domain) == 0",
    );
    let s = applies(&s, "module auth\nenable identities.Corp");
    refuses(
        &s,
        "module auth\nrevert 0001",
        "migration 0002 depends on it",
    );
}

#[test]
fn revert_of_a_merge_splits_back_by_the_discriminator() {
    let s = applies(
        &running(),
        "module auth
add type Provider = Google | Apple
merge constructor Identity.(Google | Apple) into Social
  { provider: Provider, sub: Text }
  by match
    | Google g -> Social { provider: Google, sub: g.gid }
    | Apple a  -> Social { provider: Apple, sub: a.sub }
  arm display_name: Social s -> s.sub
  arm login_present: Social s -> scalar_len(s.sub) == 0",
    );
    let s = applies(&s, "module auth\nrevert 0001");
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(
        t.ends_with("| Google { gid: Text } | Apple { sub: Text }"),
        "{t}"
    );
    let r = flat(&decl(&s, "relation", "identities"));
    assert!(r.contains("unique (gid) in Google"), "{r}");
    assert!(r.contains("unique (sub) in Apple"), "{r}");
    assert!(s.find("type", "auth", "Provider").is_none());
    let f = flat(&decl(&s, "fn", "display_name"));
    assert!(
        f.contains("| Google g -> g.gid") && f.contains("| Apple a -> a.sub"),
        "{f}"
    );
}

#[test]
fn revert_of_a_split_merges_back() {
    let base = source(
        "module auth
type Provider = Google | Apple
type Identity { id: Key identities } = Mail { email: Text } | OAuth { provider: Provider, sub: Text }
relation identities : Identity key id generate uuidv7 unique (provider, sub) in OAuth
fn name (i: Identity) -> Text = match i | Mail m -> m.email | OAuth o -> o.sub",
    );
    let s = applies(
        &base,
        "module auth
split constructor Identity.OAuth into Google { gid: Text } | Apple { sub: Text }
  by match
    | OAuth { provider: Google, sub } -> Google { gid: sub }
    | OAuth { provider: Apple, sub }  -> Apple { sub: sub }
  unique (gid) in Google
  unique (sub) in Apple
  arm name: Google g -> g.gid
  arm name: Apple a -> a.sub",
    );
    let s = applies(&s, "module auth\nrevert 0001");
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(
        t.ends_with("| OAuth { provider: Provider, sub: Text }"),
        "{t}"
    );
    let r = flat(&decl(&s, "relation", "identities"));
    assert!(r.contains("unique (provider, sub) in OAuth"), "{r}");
    assert!(flat(&decl(&s, "fn", "name")).contains("| OAuth o -> o.sub"));
}

#[test]
fn revert_of_a_lossy_split_is_refused() {
    let base = source(
        "module auth
type Identity { id: Key identities } = Mail { email: Text } | OAuth { kind: Text, sub: Text }
relation identities : Identity key id generate uuidv7",
    );
    let s = applies(
        &base,
        "module auth
split constructor Identity.OAuth into Google { gid: Text } | Apple { sub: Text }
  by match
    | OAuth { kind: \"g\", sub } -> Google { gid: sub }
    | OAuth o -> Apple { sub: o.sub }",
    );
    refuses(
        &s,
        "module auth\nrevert 0001",
        "is not invertible, so it has no inverse",
    );
}

#[test]
fn revert_of_a_split_with_a_product_no_arm_reaches_is_refused_naming_it() {
    let base = source(
        "module pay
type Payment { id: Key payments } = Cash { amount: I64 } | Card { last4: Text }
relation payments : Payment key id generate sequence",
    );
    let s = applies(
        &base,
        "module pay
split constructor Payment.Card into Visa { last4: Text } | Other { note: Text }
  by match
    | Card { last4 } -> Visa { last4: last4 }",
    );
    refuses(
        &s,
        "module pay\nrevert 0001",
        "the split of `Card` routes no row to `Other`, so it is lossy and has no inverse",
    );
}

#[test]
fn revert_of_a_nest_unnests() {
    let s = applies(
        &running(),
        "module auth
add type OAuthLogin = Google { gid: Text } | Apple { sub: Text }
nest constructor Identity.(Google | Apple) into Social { login: OAuthLogin }
  arm display_name: Social s -> match s.login | Google g -> g.gid | Apple a -> a.sub
  arm login_present: Social s ->
    match s.login | Google g -> scalar_len(g.gid) == 0 | Apple a -> scalar_len(a.sub) == 0",
    );
    let s = applies(&s, "module auth\nrevert 0001");
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(
        t.ends_with("| Google { gid: Text } | Apple { sub: Text }"),
        "{t}"
    );
    assert!(s.find("type", "auth", "OAuthLogin").is_none());
}

#[test]
fn revert_of_a_remove_puts_the_constructor_back() {
    let s = applies(&running(), "module auth\ndeprecate identities.Apple");
    let s = applies(
        &s,
        "module auth
remove constructor Identity.Apple
redefine fn display_name (i: Identity) -> Text = match i
  | EmailPass e -> e.email
  | Google g    -> g.gid",
    );
    let s = applies(&s, "module auth\nrevert 0002");
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(
        t.ends_with("| Google { gid: Text } | Apple { sub: Text }"),
        "{t}"
    );
    assert!(decl(&s, "constraint", "login_present").contains("Apple a"));
    assert!(decl(&s, "fn", "display_name").contains("Apple a"));
    assert_eq!(Stage::Deprecated, stage(&s, "identities", "Apple"));
}

#[test]
fn revert_of_a_drop_relation_restores_its_sets_and_states() {
    let s = applies(&with_badges(), "module auth\ndrop relation post_badges");
    let s = applies(&s, "module auth\nrevert 0001");
    assert!(s.find("relation", "auth", "post_badges").is_some());
    assert!(s.find("set", "auth", "post_badges.Simple").is_some());
    assert_eq!(Stage::Enabled, stage(&s, "post_badges", "Plain"));
}

#[test]
fn revert_of_a_rekey_rekeys_back() {
    let base = source(
        "module billing
type Price = { id: Key prices, sku: Text, during: Span Instant }
relation prices : Price key id generate sequence
type Order = { id: Key orders, price: Key prices }
relation orders : Order key id generate sequence",
    );
    let s = migrate(
        &base,
        "module billing\nrekey relation prices key id\n  carry Order.price",
    )
    .unwrap();
    let s = migrate(&s, "module billing\nrevert 0001").unwrap_or_else(|e| panic!("{e:?}"));
    let i = s.find("relation", "billing", "prices").unwrap();
    assert_eq!(
        "relation prices : Price key id generate sequence",
        s.decls[i].text
    );
}

#[test]
fn revert_of_a_revert_applies_the_migration_again() {
    let s = applies(&running(), "module auth\nadd index (user) in identities");
    let s = applies(&s, "module auth\nrevert 0001");
    let s = applies(&s, "module auth\nrevert 0002");
    assert!(decl(&s, "relation", "identities").contains("index (user)"));
}

#[test]
fn revert_of_a_field_rename_and_a_move_restores_the_field() {
    let s = applies(
        &running(),
        "module auth\nrename field Identity.EmailPass.hash to pw_hash",
    );
    let s = applies(&s, "module auth\nrevert 0001");
    assert_eq!(running().decls, s.decls);
    let s = applies(
        &running(),
        "module auth\nmove field Identity.EmailPass.email to Identity fill match | Google g -> g.gid | Apple a -> a.sub",
    );
    let s = applies(&s, "module auth\nrevert 0001");
    let t = flat(&decl(&s, "type", "Identity"));
    assert!(
        t.contains("EmailPass { email: Text, hash: Bytes }"),
        "the field keeps its place: {t}"
    );
}

#[test]
fn revert_of_a_dropped_record_field_puts_it_back_in_place() {
    let s = applies(&running(), "module auth\ndrop field User.name");
    let s = applies(&s, "module auth\nrevert 0001\n  fill User.name = \"\"");
    assert_eq!(
        "type User = { id: Key users, name: Text }",
        decl(&s, "type", "User")
    );
}

#[test]
fn a_record_gains_a_constructor_as_any_sum() {
    let s = applies(
        &running(),
        "module auth\nadd type Point = { x: I64, y: I64 }",
    );
    let s = applies(
        &s,
        "module auth\nadd constructor Point.Polar { r: F64, theta: F64 }",
    );
    assert_eq!(
        "type Point = Point { x: I64, y: I64 }\n  | Polar { r: F64, theta: F64 }",
        decl(&s, "type", "Point")
    );
    refuses(
        &s,
        "module auth\nadd constructor Point.Point",
        "`Point` is already a constructor of `Point`",
    );
}

#[test]
fn a_record_keyed_by_its_fields_needs_its_key_in_a_header_once_it_has_two_constructors() {
    refuses(
        &running(),
        "module auth\nadd constructor User.Admin { level: I32 }",
        "key field `id` is not a header field",
    );
}

#[test]
fn a_relation_added_later_takes_the_type_state_of_a_constructor() {
    let s = applies(
        &with_badges(),
        "module auth\nadd constructor Badge.Emoji { glyph: Text }\nadd relation team_badges : Badge key name",
    );
    assert_eq!(Stage::Declared, stage(&s, "team_badges", "Emoji"));
    assert_eq!(Stage::Enabled, stage(&s, "team_badges", "Plain"));
    let s = applies(
        &s,
        "module auth\nenable Badge.Emoji\nadd relation more_badges : Badge key name",
    );
    assert_eq!(Stage::Enabled, stage(&s, "more_badges", "Emoji"));
}

fn ids_of(s: &Source) -> Schema {
    s.schema().unwrap_or_else(|e| panic!("builds: {e:?}"))
}

fn ty(schema: &Schema, name: &str) -> TypeId {
    TypeId(
        schema
            .types
            .iter()
            .position(|t| t.name == name)
            .expect("a type") as u32,
    )
}

fn rel(schema: &Schema, name: &str) -> RelId {
    let m = schema.module_named("auth").unwrap();
    match schema.module(m).values.get(name) {
        Some(&ValueDef::Relation(r)) => r,
        _ => panic!("no relation `{name}`"),
    }
}

#[test]
fn an_added_constructor_takes_the_next_shared_id() {
    let base = ids_of(&running());
    let next = base
        .ids
        .relations
        .values()
        .chain(base.ids.types.values())
        .chain(base.ids.ctors.values())
        .copied()
        .max()
        .unwrap()
        + 1;
    let s = applies(
        &running(),
        "module auth
add constructor Identity.Corp { domain: Text }
  arm display_name: Corp c -> c.domain
  arm login_present: Corp c -> false",
    );
    let schema = ids_of(&s);
    let identity = ty(&schema, "Identity");
    let corp = schema.ty(identity).ctor_index("Corp").unwrap();
    assert_eq!(Some(&next), schema.ids.ctors.get(&(identity, corp)));
    let apple = schema.ty(identity).ctor_index("Apple").unwrap();
    assert_eq!(
        base.ids.ctors.get(&(identity, apple)),
        schema.ids.ctors.get(&(identity, apple))
    );
}

#[test]
fn a_rename_keeps_every_id() {
    let base = ids_of(&running());
    let s = applies(
        &running(),
        "module auth
rename constructor Identity.Google to Gmail
rename type Identity to Login
rename relation identities to logins
rename set OAuth of logins to Federated
rename field Login.Gmail.gid to subject",
    );
    let schema = ids_of(&s);
    let (old_t, new_t) = (ty(&base, "Identity"), ty(&schema, "Login"));
    assert_eq!(base.ids.types.get(&old_t), schema.ids.types.get(&new_t));
    let g = base.ty(old_t).ctor_index("Google").unwrap();
    let gm = schema.ty(new_t).ctor_index("Gmail").unwrap();
    assert_eq!(
        base.ids.ctors.get(&(old_t, g)),
        schema.ids.ctors.get(&(new_t, gm))
    );
    assert_eq!(
        base.ids.fields.get(&(old_t, Some(g), "gid".to_string())),
        schema
            .ids
            .fields
            .get(&(new_t, Some(gm), "subject".to_string()))
    );
    let (old_r, new_r) = (rel(&base, "identities"), rel(&schema, "logins"));
    assert_eq!(
        base.ids.relations.get(&old_r),
        schema.ids.relations.get(&new_r)
    );
    assert_eq!(
        base.ids.sets.get(&(old_r, "OAuth".to_string())),
        schema.ids.sets.get(&(new_r, "Federated".to_string()))
    );
    let gid_unique = |s: &Schema, r: RelId| {
        (0..s.relation(r).indexes.len()).find(|&i| s.relation(r).indexes[i].scope == Some(vec![1]))
    };
    let (i, j) = (
        gid_unique(&base, old_r).unwrap(),
        gid_unique(&schema, new_r).unwrap(),
    );
    assert_eq!(
        base.ids.indexes.get(&(old_r, i)),
        schema.ids.indexes.get(&(new_r, j))
    );
}

#[test]
fn ids_are_never_reused() {
    let s = applies(&running(), "module auth\nadd index (user) in identities");
    let first = ids_of(&s);
    let r = rel(&first, "identities");
    let added = first.ids.indexes[&(r, first.relation(r).indexes.len() - 1)];
    assert!(first.ids.implied.keys().all(|(i, _, _)| *i != r));
    let s = applies(&s, "module auth\ndrop index (user) in identities");
    let dropped = ids_of(&s);
    let r = rel(&dropped, "identities");
    let implied = dropped.ids.implied[&(r, None, "user".to_string())];
    assert_eq!(added + 1, implied);
    let s = applies(&s, "module auth\nadd index (user) in identities");
    let again = ids_of(&s);
    let r = rel(&again, "identities");
    let readded = again.ids.indexes[&(r, again.relation(r).indexes.len() - 1)];
    assert_eq!(implied + 1, readded);
}

#[test]
fn a_dropped_unique_leaves_the_others_their_ids() {
    let base = ids_of(&running());
    let r = rel(&base, "identities");
    let sub = base.ids.indexes[&(r, 2)];
    let s = applies(
        &running(),
        "module auth\ndrop unique (email) in identities.EmailPass",
    );
    let schema = ids_of(&s);
    assert_eq!(
        Some(&sub),
        schema.ids.indexes.get(&(rel(&schema, "identities"), 1))
    );
}

#[test]
fn a_constructor_put_back_by_revert_takes_a_new_id() {
    let base = ids_of(&running());
    let s = applies(&running(), "module auth\ndeprecate identities.Apple");
    let s = applies(
        &s,
        "module auth
remove constructor Identity.Apple
redefine fn display_name (i: Identity) -> Text = match i
  | EmailPass e -> e.email
  | Google g    -> g.gid",
    );
    let s = applies(&s, "module auth\nrevert 0002");
    let schema = ids_of(&s);
    let t = ty(&schema, "Identity");
    let apple = schema.ty(t).ctor_index("Apple").unwrap();
    assert_ne!(
        base.ids.ctors.get(&(t, apple)),
        schema.ids.ctors.get(&(t, apple))
    );
}

#[test]
fn the_schema_carries_write_states() {
    let s = applies(
        &with_badges(),
        "module auth
add constructor Badge.Emoji { glyph: Text }
enable post_badges.Emoji",
    );
    let schema = ids_of(&s);
    let badge = ty(&schema, "Badge");
    let emoji = schema.ty(badge).ctor_index("Emoji").unwrap();
    assert_eq!(
        Stage::Enabled,
        schema.stage(rel(&schema, "post_badges"), emoji)
    );
    assert_eq!(
        Stage::Declared,
        schema.stage(rel(&schema, "user_badges"), emoji)
    );
    assert_eq!(Stage::Declared, schema.value_stage(badge, emoji));
    let s = applies(&s, "module auth\ndeprecate post_badges.Emoji");
    let schema = ids_of(&s);
    assert_eq!(
        Stage::Deprecated,
        schema.stage(rel(&schema, "post_badges"), emoji)
    );
}

fn field_id(s: &Schema, t: &str, c: Option<&str>, f: &str) -> Option<u64> {
    let def = ty(s, t);
    let c = c.map(|c| s.ty(def).ctor_index(c).expect("a constructor"));
    s.ids.fields.get(&(def, c, f.to_string())).copied()
}

#[test]
fn a_moved_field_keeps_its_id() {
    let s = applies(
        &running(),
        "module auth\nadd field Identity.EmailPass.verified: Bool fill false",
    );
    let added = field_id(&ids_of(&s), "Identity", Some("EmailPass"), "verified").unwrap();
    let s = applies(
        &s,
        "module auth
move field Identity.EmailPass.verified to Identity fill match
  | Google g -> true
  | Apple a  -> true",
    );
    let up = ids_of(&s);
    assert_eq!(Some(added), field_id(&up, "Identity", None, "verified"));
    let next = up.ids.fields.values().max().copied().unwrap() + 1;
    let s = applies(
        &s,
        "module auth\nmove field Identity.verified to Identity.(Apple | Google | EmailPass)",
    );
    let down = ids_of(&s);
    assert_eq!(
        Some(added),
        field_id(&down, "Identity", Some("EmailPass"), "verified")
    );
    assert_eq!(
        Some(next),
        field_id(&down, "Identity", Some("Google"), "verified")
    );
    assert_eq!(
        Some(next + 1),
        field_id(&down, "Identity", Some("Apple"), "verified")
    );
}

#[test]
fn a_rescoped_index_is_a_new_index() {
    let base = ids_of(&running());
    let r = rel(&base, "identities");
    let next = base
        .ids
        .indexes
        .values()
        .chain(base.ids.keys.values())
        .chain(base.ids.implied.values())
        .max()
        .copied()
        .unwrap()
        + 1;
    let gid = |s: &Schema| {
        let r = rel(s, "identities");
        (0..s.relation(r).indexes.len())
            .map(|i| s.ids.indexes[&(r, i)])
            .collect::<Vec<_>>()
    };
    let before = gid(&base);
    let s = applies(
        &running(),
        "module auth
add type Provider = Google | Apple
merge constructor Identity.(Google | Apple) into Social
  { provider: Provider, sub: Text }
  by match
    | Google g -> Social { provider: Google, sub: g.gid }
    | Apple a  -> Social { provider: Apple, sub: a.sub }
  arm display_name: Social s -> s.sub
  arm login_present: Social s -> scalar_len(s.sub) == 0",
    );
    let after = ids_of(&s);
    let ids = gid(&after);
    assert!(
        ids.contains(&base.ids.indexes[&(r, 0)]),
        "{before:?} {ids:?}"
    );
    assert!(ids.contains(&next), "{before:?} {ids:?}");
    assert!(
        !ids.contains(&base.ids.indexes[&(r, 1)]),
        "{before:?} {ids:?}"
    );
}

#[test]
fn a_stored_view_has_a_row_type() {
    let s = applies(
        &running(),
        "module auth
define fn per_user () -> Set ({ user: Key users, logins: I64 }) =
  from u in users select { user: key u, logins: count(i for i in identities where i.user == key u) }
store per_user
  key user",
    );
    let schema = ids_of(&s);
    let Some(&ValueDef::Function(f)) = schema
        .module(schema.module_named("auth").unwrap())
        .values
        .get("per_user")
    else {
        panic!("no view");
    };
    let view = schema.ids.views[&f];
    assert_eq!(Some(&(view + 1)), schema.ids.view_types.get(&f));
    assert_eq!(Some(&(view + 2)), schema.ids.view_ctors.get(&f));
    let user = schema.ids.view_fields[&(f, "user".to_string())];
    assert_eq!(
        Some(&(user + 1)),
        schema.ids.view_fields.get(&(f, "logins".to_string()))
    );
}

#[test]
fn a_reverted_move_or_rename_keeps_its_ids() {
    let base = ids_of(&running());
    let gid = field_id(&base, "Identity", Some("Google"), "gid");
    let s = applies(
        &running(),
        "module auth\nrename field Identity.Google.gid to subject",
    );
    let s = applies(&s, "module auth\nrevert 0001");
    assert_eq!(
        gid,
        field_id(&ids_of(&s), "Identity", Some("Google"), "gid")
    );
    let s = applies(
        &running(),
        "module auth
move field Identity.EmailPass.email to Identity fill match
  | Google g -> g.gid
  | Apple a  -> a.sub",
    );
    let email = field_id(&base, "Identity", Some("EmailPass"), "email");
    assert_eq!(email, field_id(&ids_of(&s), "Identity", None, "email"));
    let s = applies(&s, "module auth\nrevert 0001");
    assert_eq!(
        email,
        field_id(&ids_of(&s), "Identity", Some("EmailPass"), "email")
    );
}

#[test]
fn a_stored_view_of_a_declared_type_has_no_row_type_of_its_own() {
    let s = applies(
        &running(),
        "module auth
define fn named () -> Set User = from u in users select u
store named
  key id",
    );
    let schema = ids_of(&s);
    let Some(&ValueDef::Function(f)) = schema
        .module(schema.module_named("auth").unwrap())
        .values
        .get("named")
    else {
        panic!("no view");
    };
    assert!(schema.ids.views.contains_key(&f));
    assert_eq!(None, schema.ids.view_types.get(&f));
    assert_eq!(None, schema.ids.view_ctors.get(&f));
}

/// The canonical token form of `operand` in the last migration of `src`, names as it resolves them.
fn logged_term(src: &Source, operand: &str) -> String {
    let logged = src.log.last().expect("a migration");
    let names = logged
        .names()
        .unwrap_or_else(|ds| panic!("the names resolve: {ds:?}"));
    let at = logged
        .text
        .find(operand)
        .unwrap_or_else(|| panic!("no {operand:?} in {:?}", logged.text));
    crate::canon::canonical_term(&logged.text, Span::new(at, at + operand.len()), &names)
}

#[test]
fn the_names_of_a_script_write_resolve() {
    let write = "update i in identities.Google where ends_with(i.gid, \"@corp\")
  set Corp { i with domain: i.gid }";
    let s = applies(
        &running(),
        &format!(
            "module auth
add constructor Identity.Corp {{ domain: Text }}
  arm display_name: Corp c -> c.domain
  arm login_present: Corp c -> scalar_len(c.domain) == 0
enable identities.Corp
script
  {write}"
        ),
    );
    assert_eq!(
        "update i in auth::identities . auth::Google where ends_with ( i . gid , \"@corp\" ) set auth::Corp { i with domain : i . gid }",
        logged_term(&s, write)
    );
}

#[test]
fn the_names_of_grant_and_revoke_targets_resolve() {
    let s = applies(
        &running(),
        "module auth\ngrant read identities.(EmailPass | Google) (user) to support",
    );
    assert_eq!(
        "auth::identities . ( auth::EmailPass | auth::Google )",
        logged_term(&s, "identities.(EmailPass | Google)")
    );
    let s = applies(
        &s,
        "module auth\nrevoke read identities.(EmailPass | Google) (user) from support",
    );
    assert_eq!(
        "auth::identities . ( auth::EmailPass | auth::Google )",
        logged_term(&s, "identities.(EmailPass | Google)")
    );
    let s = applies(&running(), "module auth\ngrant call fn display_name to web");
    assert_eq!("auth::display_name", logged_term(&s, "display_name"));
}

#[test]
fn the_names_of_a_revert_fill_resolve() {
    let s = applies(&running(), "module auth\ndefine fn blank () -> Text = \"\"");
    let s = applies(&s, "module auth\ndrop field User.name");
    let s = applies(&s, "module auth\nrevert 0002\n  fill User.name = blank()");
    assert_eq!("auth::blank ( )", logged_term(&s, "blank()"));
}

#[test]
fn a_reference_cycle_is_declared_within_one_migration() {
    let s = applies(
        &running(),
        "module auth
add type Team = { id: Key teams, name: Text, lead: Key members }
add relation teams : Team key id generate sequence
add type Member = { id: Key members, team: Key teams }
add relation members : Member key id generate sequence",
    );
    assert!(s.check().is_ok());
}

#[test]
fn only_a_key_type_names_a_relation_a_later_operator_adds() {
    refuses(
        &running(),
        "module auth
add relation teams : Team key id generate sequence
add type Team = { id: Key teams, name: Text }",
        "Team",
    );
    applies(
        &running(),
        "module auth
add type Team = { id: Key teams, lead: Key members.Admin }
add relation teams : Team key id generate sequence
add type Member { id: Key members } = Plain { name: Text } | Admin { level: I32 }
add relation members : Member key id generate sequence",
    );
}

#[test]
fn a_name_outside_a_key_type_types_against_the_operators_before_it() {
    refuses(
        &running(),
        "module auth
add type Pair = { left: Later, right: Text }
add type Later = { n: I64 }",
        "Later",
    );
    refuses(
        &running(),
        "module auth
define fn team_count () -> I64 = count(t for t in teams)
add type Team = { id: Key teams }
add relation teams : Team key id generate sequence
add unique (id) in teams",
        "teams",
    );
    refuses(
        &running(),
        "module auth
define fn nick (u: User) -> Text = u.nickname
add field User.nickname: Text fill \"\"",
        "nickname",
    );
    let counted = applies(
        &running(),
        "module auth\ndefine fn team_count () -> I64 = 0",
    );
    refuses(
        &counted,
        "module auth
redefine fn team_count () -> I64 = count(t for t in teams)
add type Team = { id: Key teams }
add relation teams : Team key id generate sequence",
        "teams",
    );
    refuses(
        &running(),
        "module auth
add constraint no_teams = from t in teams select t
add type Team = { id: Key teams }
add relation teams : Team key id generate sequence",
        "teams",
    );
    refuses(
        &running(),
        "module auth
add field User.pair: Pair fill Pair { n: 0 }
add type Pair = { n: I64 }",
        "Pair",
    );
}

#[test]
fn a_key_type_in_a_signature_names_a_relation_a_later_operator_adds() {
    let s = applies(
        &running(),
        "module auth
define fn same (t: Key teams) -> Key teams = t
add type Team = { id: Key teams }
add relation teams : Team key id generate sequence",
    );
    assert!(s.check().is_ok());
}

#[test]
fn declarations_in_any_order_give_the_same_schema() {
    let decls = [
        "add type Team = { id: Key teams, name: Text, lead: Key members }",
        "add relation teams : Team key id generate sequence",
        "add type Member = { id: Key members, team: Key teams }",
        "add relation members : Member key id generate sequence",
    ];
    let orders: [[usize; 4]; 4] = [[0, 1, 2, 3], [2, 3, 0, 1], [0, 2, 1, 3], [2, 0, 3, 1]];
    let mut printed = Vec::new();
    for order in orders {
        let text = std::iter::once("module auth")
            .chain(order.iter().map(|&i| decls[i]))
            .collect::<Vec<_>>()
            .join("\n");
        let s = applies(&running(), &text);
        let mut ds: Vec<String> = ["Team", "Member"]
            .iter()
            .map(|t| decl(&s, "type", t))
            .chain(["teams", "members"].iter().map(|r| decl(&s, "relation", r)))
            .collect();
        ds.sort();
        printed.push((order, ds));
    }
    for (order, ds) in &printed[1..] {
        assert_eq!(&printed[0].1, ds, "order {order:?}");
    }
}

#[test]
fn revert_of_a_reference_cycle_drops_both_relations() {
    let s = applies(
        &running(),
        "module auth
add type Team = { id: Key teams, name: Text, lead: Key members }
add relation teams : Team key id generate sequence
add type Member = { id: Key members, team: Key teams }
add relation members : Member key id generate sequence",
    );
    let s = applies(&s, "module auth\nrevert 0001");
    assert!(s.find("type", "auth", "Team").is_none());
    assert!(s.find("relation", "auth", "members").is_none());
}

#[test]
fn unnest_rescopes_a_unique_written_with_its_relation() {
    let base = source(
        "module auth
type Customer = { id: Key customers }
relation customers : Customer key id generate sequence
type Payment { id: Key payments, customer: Key customers } = Card { last4: Text } | Invoice { due: Text }
relation payments : Payment key id generate sequence",
    );
    let s = applies(&base, "module auth\nadd unique (last4) in payments.Card");
    let s = applies(
        &s,
        "module auth
add type Method = Card { last4: Text } | Invoice { due: Text }
nest constructor Payment.(Card | Invoice) into Held { method: Method }",
    );
    let r = flat(&decl(&s, "relation", "payments"));
    assert!(r.contains("unique (method.Card.last4) in Held"), "{r}");
    let s = applies(&s, "module auth\nunnest constructor Payment.Held");
    let r = flat(&decl(&s, "relation", "payments"));
    assert!(r.contains("unique (last4) in Card"), "{r}");
}

#[test]
fn a_relation_a_later_operator_adds_is_no_relation_before_it() {
    let text = "module auth
add type Team = { id: Key teams }
add relation teams : Team key id generate sequence";
    let m = parse_migration(text, Options::default())
        .ok()
        .expect("the migration parses");
    let mut s = running();
    s.expecting("auth", &m.operators);
    s.apply("auth", &m.operators[0], text)
        .unwrap_or_else(|e| panic!("add type refused: {e:?}"));
    let schema = s.schema().unwrap_or_else(|e| panic!("{e:?}"));
    assert!(schema.relations.iter().all(|r| r.name != "teams"));
    let team = schema
        .types
        .iter()
        .find(|t| t.name == "Team")
        .expect("Team is declared");
    let id = team.field(0, "id").expect("Team has field id");
    assert_eq!("Key teams", schema.show_ty(&id.ty));
}

#[test]
fn an_index_over_a_key_names_a_relation_a_later_operator_adds() {
    let s = applies(
        &running(),
        "module auth
add type Team = { id: Key teams, name: Text, lead: Key members }
add type Member = { id: Key members, team: Key teams }
add relation teams : Team key id generate sequence index (lead)
add relation members : Member key id generate sequence index (team)",
    );
    assert!(s.check().is_ok());
}
