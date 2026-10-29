use super::*;

const GRANTS: &str = r"
module auth
type User = { id: Key users, name: Text }
type Provider = Google | Apple
type OAuthLogin = Google { gid: Text } | Apple { sub: Text }
type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes }
  | Google    { gid: Text }
  | Apple     { sub: Text }
relation users : User key id generate sequence
relation identities : Identity key id generate uuidv7
set OAuth of identities = Google | Apple
";

const OAUTH: &str = r"
module auth
type User = { id: Key users, name: Text }
type Provider = Google | Apple
type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes }
  | OAuth     { provider: Provider, sub: Text }
relation users : User key id generate sequence
relation identities : Identity key id generate uuidv7
set Federated of identities = OAuth
";

const SPLIT: &str = "module auth
split constructor Identity.OAuth into Google { gid: Text } | Apple { sub: Text }
  by match
    | OAuth { provider: Google, sub } -> Google { gid: sub }
    | OAuth { provider: Apple, sub }  -> Apple { sub: sub }";

const MERGE: &str = "module auth
merge constructor Identity.(Google | Apple) into Social
  { provider: Provider, sub: Text }
  by match
    | Google g -> Social { provider: Google, sub: g.gid }
    | Apple a  -> Social { provider: Apple, sub: a.sub }";

const NEST: &str =
    "module auth\nnest constructor Identity.(Google | Apple) into Social { login: OAuthLogin }";

/// Returns each log grant of `src`, its ids shown as the names of what they name: the role, the
/// privilege, the relation, the constructors (`*` for the whole relation), and for `read` and
/// `update` the fields, `listed` or `frozen`.
fn held(src: &Source) -> Vec<String> {
    let name = |kind: &str, id: u64| {
        let path = src
            .ids
            .iter()
            .find(|(p, x)| **x == id && p.starts_with(kind))
            .map_or_else(|| format!("#{id}"), |(p, _)| p.clone());
        let local = path.split_once("::").map_or(path.as_str(), |(_, l)| l);
        match kind {
            "relation " | "fn " => local.to_string(),
            _ => local.split_once('.').map_or(local, |(_, l)| l).to_string(),
        }
    };
    let target = |t: &GrantTarget| {
        let sel = match &t.sel {
            GrantSel::Whole => "*".to_string(),
            GrantSel::Set(s) => format!("set {}", name("set ", *s)),
            GrantSel::Ctors(cs) => {
                let mut ns: Vec<String> = cs.iter().map(|&c| name("ctor ", c)).collect();
                ns.sort();
                format!("({})", ns.join(" | "))
            }
        };
        format!("{} {sel}", name("relation ", t.relation))
    };
    let fields = |fs: &GrantFields| {
        let mut ns: Vec<String> = fs.ids.iter().map(|&f| name("field ", f)).collect();
        ns.sort();
        let how = if fs.listed { "listed" } else { "frozen" };
        format!("{how} [{}]", ns.join(", "))
    };
    src.grants
        .iter()
        .map(|g| {
            let p = match &g.privilege {
                Granted::Read(t, fs) => format!("read {} {}", target(t), fields(fs)),
                Granted::Update(t, fs) => format!("update {} {}", target(t), fields(fs)),
                Granted::Insert(t) => format!("insert {}", target(t)),
                Granted::Delete(t) => format!("delete {}", target(t)),
                Granted::Retag(a, b) => format!("retag {} -> {}", target(a), target(b)),
                Granted::CallFn(f) => format!("call {}", name("fn ", *f)),
            };
            let option = if g.grant_option {
                " with grant option"
            } else {
                ""
            };
            format!("{} {p}{option}", g.role)
        })
        .collect()
}

fn grants() -> Source {
    source(GRANTS)
}

fn oauth() -> Source {
    source(OAUTH)
}

/// Assert that reverting the last migration of `after` gives back the grants of `before`.
fn reverts_to(before: &Source, after: &Source) {
    let last = after.log.last().expect("a migration").number;
    let s = applies(after, &format!("module auth\nrevert {last:04}"));
    assert_eq!(held(before), held(&s), "revert {last:04}");
}

#[test]
fn a_grant_on_listed_constructors_without_fields_covers_their_fields_at_apply() {
    let s = applies(
        &grants(),
        "module auth\ngrant read identities.(EmailPass | Google) to support",
    );
    let frozen = "support read identities (EmailPass | Google) frozen [EmailPass.email, EmailPass.hash, Google.gid, id, user]";
    assert_eq!(vec![frozen], held(&s));
    let s = applies(
        &s,
        "module auth\nadd field Identity.EmailPass.verified: Bool fill false\nadd field Identity.note: Text fill \"\"",
    );
    assert_eq!(vec![frozen], held(&s));
    let s = applies(
        &s,
        "module auth\nadd constructor Identity.Github { login: Text }",
    );
    assert_eq!(vec![frozen], held(&s));
}

#[test]
fn a_grant_with_a_field_list_covers_the_listed_fields_only() {
    let s = applies(
        &grants(),
        "module auth
grant update identities.(Google | Apple) (user) to support
grant read identities.EmailPass (email, hash) to support",
    );
    let s = applies(
        &s,
        "module auth\nadd field Identity.EmailPass.verified: Bool fill false",
    );
    assert_eq!(
        vec![
            "support update identities (Apple | Google) listed [user]",
            "support read identities (EmailPass) listed [EmailPass.email, EmailPass.hash]",
        ],
        held(&s)
    );
}

#[test]
fn a_grant_on_a_named_set_without_fields_covers_a_joining_constructor_with_its_fields() {
    let s = applies(
        &grants(),
        "module auth\ngrant read identities.OAuth, update identities.(Google | Apple) to support",
    );
    assert_eq!(
        vec![
            "support read identities set OAuth frozen [Apple.sub, Google.gid, id, user]",
            "support update identities (Apple | Google) frozen [Apple.sub, Google.gid, id, user]",
        ],
        held(&s)
    );
    let s = applies(
        &s,
        "module auth\nadd constructor Identity.Github { login: Text, org: Text }\n  joins identities.OAuth",
    );
    assert_eq!(
        "set OAuth of identities = Google | Apple | Github",
        flat(&decl(&s, "set", "identities.OAuth"))
    );
    assert_eq!(
        vec![
            "support read identities set OAuth frozen [Apple.sub, Github.login, Github.org, Google.gid, id, user]",
            "support update identities (Apple | Google) frozen [Apple.sub, Google.gid, id, user]",
        ],
        held(&s)
    );
    let s = applies(
        &s,
        "module auth\nadd constructor Identity.Gitlab { login: Text }",
    );
    assert_eq!(
        "support read identities set OAuth frozen [Apple.sub, Github.login, Github.org, Google.gid, id, user]",
        held(&s)[0]
    );
}

const SHARED: &str = r"
module auth
type User = { id: Key users, name: Text }
type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes }
  | Google    { sub: Text, label: Text }
  | Apple     { sub: Text }
relation users : User key id generate sequence
relation identities : Identity key id generate uuidv7
set OAuth of identities = Google | Apple
";

#[test]
fn a_listed_grant_on_a_named_set_covers_the_listed_fields_a_joining_constructor_has() {
    let s = applies(
        &source(SHARED),
        "module auth\ngrant read identities.OAuth (user, sub) to support",
    );
    assert_eq!(
        vec!["support read identities set OAuth listed [Apple.sub, Google.sub, user]"],
        held(&s)
    );
    let s = applies(
        &s,
        "module auth\nadd constructor Identity.Github { sub: Text, label: Text }\n  joins identities.OAuth",
    );
    assert_eq!(
        vec!["support read identities set OAuth listed [Apple.sub, Github.sub, Google.sub, user]"],
        held(&s)
    );
}

#[test]
fn a_listed_grant_on_a_named_set_covers_no_field_a_joining_constructor_lacks() {
    let s = applies(
        &source(SHARED),
        "module auth\ngrant read identities.OAuth (user, sub) to support",
    );
    let s = applies(
        &s,
        "module auth\nadd constructor Identity.Github { login: Text, label: Text }\n  joins identities.OAuth",
    );
    assert_eq!(
        vec!["support read identities set OAuth listed [Apple.sub, Google.sub, user]"],
        held(&s)
    );
}

#[test]
fn a_grant_on_the_whole_relation_covers_each_constructor_added_later_with_its_fields() {
    let s = applies(&grants(), "module auth\ngrant read identities to support");
    assert_eq!(
        vec![
            "support read identities * frozen [Apple.sub, EmailPass.email, EmailPass.hash, Google.gid, id, user]"
        ],
        held(&s)
    );
    let s = applies(
        &s,
        "module auth\nadd field Identity.Google.verified: Bool fill false",
    );
    let s = applies(
        &s,
        "module auth\nadd constructor Identity.Github { login: Text, org: Text }",
    );
    assert_eq!(
        vec![
            "support read identities * frozen [Apple.sub, EmailPass.email, EmailPass.hash, Github.login, Github.org, Google.gid, id, user]"
        ],
        held(&s)
    );
    let listed = applies(
        &grants(),
        "module auth\ngrant read identities (user) to support",
    );
    let listed = applies(
        &listed,
        "module auth\nadd constructor Identity.Github { login: Text }",
    );
    assert_eq!(
        vec!["support read identities * listed [user]"],
        held(&listed)
    );
}

#[test]
fn a_revoke_without_fields_takes_a_grant_without_fields_whatever_it_froze() {
    let s = applies(
        &grants(),
        "module auth\ngrant read identities.Google to support",
    );
    let s = applies(
        &s,
        "module auth\nadd field Identity.Google.verified: Bool fill false",
    );
    let s = applies(
        &s,
        "module auth\nrevoke read identities.Google from support",
    );
    assert_eq!(Vec::<String>::new(), held(&s));
    let s = applies(
        &grants(),
        "module auth\ngrant read identities.Google (gid) to support",
    );
    let s = applies(
        &s,
        "module auth\nrevoke read identities.Google from support",
    );
    assert_eq!(
        vec!["support read identities (Google) listed [Google.gid]"],
        held(&s)
    );
}

#[test]
fn a_grant_follows_renames() {
    let s = applies(
        &grants(),
        "module auth\ngrant read identities.Google (gid), retag identities.Google -> identities.OAuth to support",
    );
    let s = applies(
        &s,
        "module auth
rename constructor Identity.Google to Gmail
rename field Identity.Gmail.gid to mail
rename set OAuth of identities to Federated
rename relation identities to logins",
    );
    assert_eq!(
        vec![
            "support read logins (Gmail) listed [Gmail.mail]",
            "support retag logins (Gmail) -> logins set Federated",
        ],
        held(&s)
    );
    let s = applies(
        &s,
        "module auth\nrevoke read logins.Gmail (mail), retag logins.Gmail -> logins.Federated from support",
    );
    assert_eq!(Vec::<String>::new(), held(&s));
}

#[test]
fn a_rename_reverts_with_its_grants() {
    let before = applies(
        &grants(),
        "module auth\ngrant read identities.Google to support",
    );
    let after = applies(
        &before,
        "module auth\nrename constructor Identity.Google to Gmail\nrename field Identity.Gmail.gid to mail",
    );
    reverts_to(&before, &after);
}

#[test]
fn a_split_gives_each_product_the_grants_on_the_fields_it_carries_whole() {
    let s = applies(
        &oauth(),
        "module auth
grant read identities.OAuth, insert identities.OAuth to support
grant read identities.Federated (user, sub) to web
grant read identities to audit",
    );
    let s = applies(&s, SPLIT);
    assert_eq!(
        vec![
            "support read identities (Apple | Google) frozen [Apple.sub, Google.gid, id, user]",
            "support insert identities (Apple | Google)",
            "web read identities set Federated listed [Apple.sub, Google.gid, user]",
            "audit read identities * frozen [Apple.sub, EmailPass.email, EmailPass.hash, Google.gid, id, user]",
        ],
        held(&s)
    );
}

#[test]
fn a_split_does_not_cover_a_field_its_routing_computes() {
    let s = applies(
        &oauth(),
        "module auth\ngrant read identities.OAuth to support\ngrant read identities.EmailPass to web",
    );
    let s = applies(
        &s,
        "module auth
split constructor Identity.OAuth into Google { gid: Text } | Apple { sub: Text, label: Text }
  by match
    | OAuth { provider: Google, sub } -> Google { gid: \"google\" }
    | OAuth { provider: Apple, sub }  -> Apple { sub: sub, label: sub }",
    );
    assert_eq!(
        vec![
            "support read identities (Apple | Google) frozen [Apple.label, Apple.sub, id, user]",
            "web read identities (EmailPass) frozen [EmailPass.email, EmailPass.hash, id, user]",
        ],
        held(&s)
    );
}

#[test]
fn a_split_does_not_cover_a_field_one_routing_carries_and_another_computes() {
    let s = applies(
        &oauth(),
        "module auth\ngrant read identities.OAuth to support",
    );
    let s = applies(
        &s,
        "module auth
split constructor Identity.OAuth into Google { gid: Text } | Apple { sub: Text }
  by match
    | OAuth { provider: Google, sub } if sub == \"\" -> Google { gid: \"none\" }
    | OAuth { provider: Google, sub } -> Google { gid: sub }
    | OAuth { provider: Apple, sub }  -> Apple { sub: sub }",
    );
    assert_eq!(
        vec!["support read identities (Apple | Google) frozen [Apple.sub, id, user]"],
        held(&s)
    );
}

#[test]
fn a_split_reverts_with_its_grants() {
    let before = applies(
        &oauth(),
        "module auth
grant read identities.OAuth, delete identities.(EmailPass | OAuth) to support
grant read identities to audit with grant option
grant retag identities.EmailPass -> identities.Federated to web",
    );
    let after = applies(&before, SPLIT);
    reverts_to(&before, &after);
}

#[test]
fn a_merge_carries_a_grant_every_source_holds_with_the_same_fields() {
    let s = applies(
        &grants(),
        "module auth
grant read identities.(Google | Apple), delete identities.OAuth to support
grant read identities.(EmailPass | Google | Apple) (user) to web
grant read identities to audit",
    );
    let s = applies(&s, MERGE);
    assert_eq!(
        vec![
            "support read identities (Social) frozen [Social.provider, Social.sub, id, user]",
            "support delete identities set OAuth",
            "web read identities (EmailPass | Social) listed [Social.provider, user]",
            "audit read identities * frozen [EmailPass.email, EmailPass.hash, Social.provider, Social.sub, id, user]",
        ],
        held(&s)
    );
}

#[test]
fn a_merge_does_not_cover_the_discriminator_for_a_grant_missing_a_source() {
    let s = applies(
        &grants(),
        "module auth\ngrant read identities.(EmailPass | Google) to support",
    );
    refuses(
        &s,
        MERGE,
        "the merge of `Google`, `Apple` into `Social` cannot carry the `read` grant of `support` on `identities`",
    );
    let s = applies(
        &s,
        &format!("{MERGE}\ngrant read identities.Social (sub) to support"),
    );
    assert_eq!(
        vec![
            "support read identities (EmailPass) frozen [EmailPass.email, EmailPass.hash, id, user]",
            "support read identities (Social) listed [Social.sub]",
        ],
        held(&s)
    );
}

#[test]
fn a_merge_is_refused_while_a_source_is_granted_alone() {
    for (grant, kind) in [
        ("grant read identities.Google to support", "read"),
        (
            "grant insert identities.(EmailPass | Apple) to support",
            "insert",
        ),
        (
            "grant retag identities.Google -> identities.Apple to support",
            "retag",
        ),
    ] {
        let s = applies(&grants(), &format!("module auth\n{grant}"));
        refuses(
            &s,
            MERGE,
            &format!(
                "the merge of `Google`, `Apple` into `Social` cannot carry the `{kind}` grant of `support` on `identities`"
            ),
        );
    }
}

#[test]
fn a_merge_is_refused_while_its_sources_are_granted_different_fields() {
    let s = applies(
        &grants(),
        "module auth\ngrant read identities.(Google | Apple) to support",
    );
    refuses(
        &s,
        "module auth
merge constructor Identity.(Google | Apple) into Social
  { provider: Provider, sub: Text }
  by match
    | Google g -> Social { provider: Google, sub: g.gid }
    | Apple a  -> Social { provider: Apple, sub: \"apple\" }",
        "the merge of `Google`, `Apple` into `Social` cannot carry the `read` grant of `support` on `identities`",
    );
}

#[test]
fn a_merge_applies_when_grant_operators_of_its_migration_state_the_merged_grants() {
    let s = applies(
        &grants(),
        "module auth\ngrant read identities.(EmailPass | Google) (user), insert identities.Apple to support",
    );
    refuses(
        &s,
        &format!("{MERGE}\ngrant read identities.Social to support"),
        "cannot carry the `insert` grant of `support`",
    );
    refuses(
        &s,
        &format!(
            "{MERGE}\ngrant read identities.Social to web\ngrant insert identities.Social to web"
        ),
        "cannot carry the `read` grant of `support`",
    );
    let s = applies(
        &s,
        &format!(
            "{MERGE}\ngrant read identities.Social (sub), insert identities.Social to support"
        ),
    );
    assert_eq!(
        vec![
            "support read identities (EmailPass) listed [user]",
            "support read identities (Social) listed [Social.sub]",
            "support insert identities (Social)",
        ],
        held(&s)
    );
}

#[test]
fn a_merge_reverts_with_its_grants() {
    let before = applies(
        &grants(),
        "module auth
grant read identities.(Google | Apple), update identities.OAuth to support
grant read identities.Google (gid) to web",
    );
    let after = applies(
        &before,
        &format!("{MERGE}\ngrant read identities.Social (sub) to web"),
    );
    reverts_to(&before, &after);
}

#[test]
fn a_nest_carries_a_grant_covering_every_field_of_every_source_to_the_nesting_field() {
    let s = applies(
        &grants(),
        "module auth
grant read identities.OAuth to support
grant read identities.(Google | Apple) (user) to web",
    );
    let s = applies(&s, NEST);
    assert_eq!(
        vec![
            "support read identities set OAuth frozen [Social.login, id, user]",
            "web read identities (Social) listed [user]",
        ],
        held(&s)
    );
}

#[test]
fn a_nest_is_refused_while_its_sources_are_granted_in_part() {
    let s = applies(
        &grants(),
        "module auth\ngrant read identities.Google to support",
    );
    refuses(
        &s,
        NEST,
        "the nest of `Google`, `Apple` into `Social` cannot carry the `read` grant of `support` on `identities`",
    );
    let s = applies(
        &grants(),
        "module auth\ngrant read identities.(Google | Apple) (gid) to support",
    );
    refuses(
        &s,
        NEST,
        "the nest of `Google`, `Apple` into `Social` cannot carry the `read` grant of `support` on `identities`",
    );
}

#[test]
fn an_unnest_carries_a_grant_on_the_nesting_field_to_every_field_of_the_products() {
    let s = applies(&grants(), NEST);
    let s = applies(
        &s,
        "module auth
grant read identities.Social to support
grant read identities.Social (user) to web",
    );
    let s = applies(&s, "module auth\nunnest constructor Identity.Social");
    assert_eq!(
        vec![
            "support read identities (Apple | Google) frozen [Apple.sub, Google.gid, id, user]",
            "web read identities (Apple | Google) listed [user]",
        ],
        held(&s)
    );
}

#[test]
fn a_nest_and_an_unnest_revert_with_their_grants() {
    let before = applies(
        &grants(),
        "module auth\ngrant read identities.OAuth, insert identities.(Google | Apple) to support",
    );
    let after = applies(&before, NEST);
    reverts_to(&before, &after);
    let before = applies(&after, "module auth\ngrant read identities.Social to web");
    let after = applies(&before, "module auth\nunnest constructor Identity.Social");
    reverts_to(&before, &after);
}

#[test]
fn a_remove_drops_the_grants_on_the_removed_constructor_or_field() {
    let s = applies(
        &grants(),
        "module auth
grant read identities.(Google | Apple), delete identities.Apple to support
grant read identities.EmailPass to web
grant retag identities.Google -> identities.Apple to ops",
    );
    let s = applies(&s, "module auth\ndeprecate identities.Apple");
    let removed = applies(&s, "module auth\nremove constructor Identity.Apple");
    assert_eq!(
        vec![
            "support read identities (Google) frozen [Google.gid, id, user]",
            "web read identities (EmailPass) frozen [EmailPass.email, EmailPass.hash, id, user]",
        ],
        held(&removed)
    );
    reverts_to(&s, &removed);
    let dropped = applies(&s, "module auth\ndrop field Identity.EmailPass.hash");
    assert_eq!(
        "web read identities (EmailPass) frozen [EmailPass.email, id, user]",
        held(&dropped)[2]
    );
}

#[test]
fn an_added_constructor_and_field_revert_with_their_grants() {
    let before = applies(&grants(), "module auth\ngrant read identities to support");
    let after = applies(
        &before,
        "module auth\nadd constructor Identity.Github { login: Text }",
    );
    reverts_to(&before, &after);
    let after = applies(
        &before,
        "module auth\nadd field Identity.Google.verified: Bool fill false",
    );
    reverts_to(&before, &after);
}

#[test]
fn a_moved_field_keeps_its_grants() {
    let s = applies(
        &grants(),
        "module auth\ngrant read identities.(EmailPass | Google | Apple) (user) to support\ngrant read identities.Google to web",
    );
    let up = applies(
        &s,
        "module auth\nmove field Identity.Google.gid to Identity fill match\n  | EmailPass e -> e.email\n  | Apple a -> a.sub",
    );
    assert_eq!(
        "web read identities (Google) frozen [gid, id, user]",
        held(&up)[1]
    );
    reverts_to(&s, &up);
    let down = applies(
        &up,
        "module auth\nmove field Identity.gid to Identity.(Google | Apple)",
    );
    assert_eq!(
        "web read identities (Google) frozen [Apple.gid, Google.gid, id, user]",
        held(&down)[1]
    );
}

#[test]
fn replaying_the_log_gives_the_same_grants() {
    let s = applies(&oauth(), "module auth\ngrant read identities to support");
    let s = applies(
        &s,
        "module auth\nadd field Identity.EmailPass.verified: Bool fill false",
    );
    let s = applies(&s, "module auth\ngrant read identities.Federated to web");
    let s = applies(&s, SPLIT);
    let mut replayed = (*s.log[0].before).clone();
    for l in &s.log {
        replayed = applies(&replayed, &l.text);
    }
    assert_eq!(s.grants, replayed.grants);
    assert_eq!(held(&s), held(&replayed));
}
