use super::*;
use crate::signature::Ids;
use crate::types::Base;

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
                Granted::Retag(a, b, except) if except.is_empty() => {
                    format!("retag {} -> {}", target(a), target(b))
                }
                Granted::Retag(a, b, except) => {
                    let mut pairs: Vec<String> = except
                        .iter()
                        .map(|&(x, y)| format!("{} -> {}", name("ctor ", x), name("ctor ", y)))
                        .collect();
                    pairs.sort();
                    format!(
                        "retag {} -> {} except [{}]",
                        target(a),
                        target(b),
                        pairs.join(", ")
                    )
                }
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
        vec![
            "support read identities set OAuth frozen [Apple.sub, Github.login, Github.org, Google.gid, id, user]",
            "support update identities (Apple | Google) frozen [Apple.sub, Google.gid, id, user]",
        ],
        held(&s)
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
    assert_eq!(
        vec![
            "`revoke read identities.Google from support` matches no grant `support` holds"
                .to_owned()
        ],
        migrate(
            &s,
            "module auth\nrevoke read identities.Google from support"
        )
        .unwrap_err(),
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
        "module auth\ngrant read identities.(Google | Apple) (Google.gid) to support",
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
        vec![
            "support read identities (Apple | Google) frozen [Apple.sub, Google.gid, id, user]",
            "support delete identities (Apple)",
            "web read identities (EmailPass) frozen [EmailPass.email, id, user]",
            "ops retag identities (Google) -> identities (Apple)",
        ],
        held(&dropped)
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
        vec![
            "support read identities (Apple | EmailPass | Google) listed [user]",
            "web read identities (Google) frozen [gid, id, user]",
        ],
        held(&up)
    );
    reverts_to(&s, &up);
    let down = applies(
        &up,
        "module auth\nmove field Identity.gid to Identity.(Google | Apple)",
    );
    assert_eq!(
        vec![
            "support read identities (Apple | EmailPass | Google) listed [user]",
            "web read identities (Google) frozen [Google.gid, id, user]",
        ],
        held(&down)
    );
    let s = applies(
        &grants(),
        "module auth\nmove field Identity.Google.gid to Identity fill match\n  | EmailPass e -> e.email\n  | Apple a -> a.sub\ngrant read identities to audit",
    );
    let down = applies(
        &s,
        "module auth\nmove field Identity.gid to Identity.(Google | Apple)",
    );
    assert_eq!(
        vec![
            "audit read identities * frozen [Apple.gid, Apple.sub, EmailPass.email, EmailPass.hash, Google.gid, id, user]"
        ],
        held(&down)
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

#[test]
fn a_revert_of_a_split_and_a_dropped_field_restores_the_grants_exactly() {
    let before = applies(
        &oauth(),
        "module auth\ngrant read identities to audit\ngrant read identities.OAuth to support",
    );
    let after = applies(&before, &format!("{SPLIT}\ndrop field Identity.Apple.sub"));
    let s = applies(
        &after,
        "module auth\nrevert 0002\n  fill Identity.Apple.sub = \"\"",
    );
    assert_eq!(
        vec![
            "audit read identities * frozen [EmailPass.email, EmailPass.hash, OAuth.provider, OAuth.sub, id, user]",
            "support read identities (OAuth) frozen [OAuth.provider, OAuth.sub, id, user]",
        ],
        held(&s)
    );
    assert_eq!(held(&before), held(&s));
}

#[test]
fn a_revert_of_a_nest_and_an_unnest_restores_the_grants_exactly() {
    let before = applies(
        &grants(),
        "module auth\ngrant read identities to audit\ngrant read identities.OAuth to support",
    );
    let after = applies(
        &before,
        &format!("{NEST}\nunnest constructor Identity.Social"),
    );
    reverts_to(&before, &after);
    assert_eq!(
        vec![
            "audit read identities * frozen [Apple.sub, EmailPass.email, EmailPass.hash, Google.gid, id, user]",
            "support read identities set OAuth frozen [Apple.sub, Google.gid, id, user]",
        ],
        held(&before)
    );
}

#[test]
fn a_revert_of_a_split_and_a_merge_of_its_products_restores_the_grants_exactly() {
    let before = applies(
        &oauth(),
        "module auth\ngrant read identities to audit\ngrant read identities.OAuth to support",
    );
    let after = applies(
        &before,
        &format!(
            "{SPLIT}
merge constructor Identity.(Google | Apple) into OAuth2
  {{ provider: Provider, sub: Text }}
  by match
    | Google g -> OAuth2 {{ provider: Google, sub: g.gid }}
    | Apple a  -> OAuth2 {{ provider: Apple, sub: a.sub }}"
        ),
    );
    assert_eq!(
        vec![
            "audit read identities * frozen [EmailPass.email, EmailPass.hash, OAuth2.provider, OAuth2.sub, id, user]",
            "support read identities (OAuth2) frozen [OAuth2.provider, OAuth2.sub, id, user]",
        ],
        held(&after)
    );
    reverts_to(&before, &after);
}

const FILL: &str = "module auth
add field Identity.EmailPass.secret: Text fill \"\"
grant read identities to audit";

#[test]
fn a_field_moved_up_is_covered_only_where_the_fill_carries_a_covered_field_whole() {
    let s = applies(
        &grants(),
        "module auth\ngrant read identities.(EmailPass | Google) to support",
    );
    let s = applies(&s, FILL);
    let secret = applies(
        &s,
        "module auth
move field Identity.Google.gid to Identity fill match
  | EmailPass e -> e.secret
  | Apple a -> a.sub",
    );
    assert_eq!(
        vec![
            "support read identities (EmailPass | Google) frozen [EmailPass.email, EmailPass.hash, id, user]",
            "audit read identities * frozen [Apple.sub, EmailPass.email, EmailPass.hash, EmailPass.secret, gid, id, user]",
        ],
        held(&secret)
    );
    reverts_to(&s, &secret);
    let email = applies(
        &s,
        "module auth
move field Identity.Google.gid to Identity fill match
  | EmailPass e -> e.email
  | Apple a -> \"apple\"",
    );
    assert_eq!(
        vec![
            "support read identities (EmailPass | Google) frozen [EmailPass.email, EmailPass.hash, gid, id, user]",
            "audit read identities * frozen [Apple.sub, EmailPass.email, EmailPass.hash, EmailPass.secret, id, user]",
        ],
        held(&email)
    );
}

const LABELS: &str = r"
module auth
type User = { id: Key users, name: Text }
type Login = Google { gid: Text, label: Text } | Apple { sub: Text, label: Text }
type GoogleLogin = Google { gid: Text, label: Text }
type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes }
  | Google    { gid: Text, label: Text }
  | Apple     { sub: Text, label: Text }
relation users : User key id generate sequence
relation identities : Identity key id generate uuidv7
";

#[test]
fn a_nest_is_refused_while_every_source_is_granted_the_same_part() {
    let s = applies(
        &source(LABELS),
        "module auth\ngrant read identities.(Google | Apple) (label) to web",
    );
    let nest =
        "module auth\nnest constructor Identity.(Google | Apple) into Social { login: Login }";
    refuses(
        &s,
        nest,
        "the nest of `Google`, `Apple` into `Social` cannot carry the `read` grant of `web` on `identities`",
    );
    let s = applies(
        &s,
        &format!("{nest}\ngrant read identities.Social (login) to web"),
    );
    assert_eq!(
        vec!["web read identities (Social) listed [Social.login]"],
        held(&s)
    );
}

#[test]
fn a_nest_of_one_source_is_refused_while_it_is_granted_in_part() {
    let s = applies(
        &source(LABELS),
        "module auth\ngrant read identities.Google (label) to web",
    );
    refuses(
        &s,
        "module auth\nnest constructor Identity.Google into Social { login: GoogleLogin }",
        "the nest of `Google` into `Social` cannot carry the `read` grant of `web` on `identities`",
    );
}

#[test]
fn a_merge_gives_update_of_the_discriminator_only_with_retags_between_the_sources() {
    let s = applies(
        &grants(),
        "module auth\ngrant update identities.(Google | Apple), read identities.(Google | Apple) to support",
    );
    let s = applies(&s, MERGE);
    assert_eq!(
        vec![
            "support update identities (Social) frozen [Social.sub, id, user]",
            "support read identities (Social) frozen [Social.provider, Social.sub, id, user]",
        ],
        held(&s)
    );
    let s = applies(
        &grants(),
        "module auth\ngrant update identities.(Google | Apple), retag identities.OAuth -> identities.OAuth to support",
    );
    let s = applies(&s, MERGE);
    assert_eq!(
        vec![
            "support update identities (Social) frozen [Social.provider, Social.sub, id, user]",
            "support retag identities set OAuth -> identities set OAuth",
        ],
        held(&s)
    );
}

#[test]
fn a_merge_refusal_names_the_privilege_role_relation_and_constructors() {
    let s = applies(
        &grants(),
        "module auth\ngrant read identities.Google to support",
    );
    assert_eq!(
        Err(vec![
            "the merge of `Google`, `Apple` into `Social` cannot carry the `read` grant of `support` on `identities`, which does not cover each of them alike; grant `read` on `identities.Social` to `support` in the same migration".to_string()
        ]),
        migrate(&s, MERGE).map(|_| ())
    );
}

#[test]
fn a_nest_gives_update_of_the_nesting_field_only_with_retags_between_the_sources() {
    for grant in [
        "grant update identities.(Google | Apple) to support",
        "grant update identities.OAuth to support",
    ] {
        let s = applies(&grants(), &format!("module auth\n{grant}"));
        let s = applies(&s, NEST);
        let target = if grant.contains("OAuth") {
            "set OAuth"
        } else {
            "(Social)"
        };
        assert_eq!(
            vec![format!(
                "support update identities {target} frozen [id, user]"
            )],
            held(&s),
            "{grant}"
        );
    }
    let s = applies(
        &grants(),
        "module auth\ngrant update identities.(Google | Apple), retag identities.OAuth -> identities.OAuth to support",
    );
    let s = applies(&s, NEST);
    assert_eq!(
        vec![
            "support update identities (Social) frozen [Social.login, id, user]",
            "support retag identities set OAuth -> identities set OAuth",
        ],
        held(&s)
    );
}

#[test]
fn a_split_leaves_out_retags_between_its_products_without_update_of_the_routed_fields() {
    let s = applies(
        &oauth(),
        "module auth\ngrant retag identities.OAuth -> identities.(OAuth | EmailPass) to ops",
    );
    let after = applies(&s, SPLIT);
    assert_eq!(
        vec![
            "ops retag identities (Apple | Google) -> identities (Apple | EmailPass | Google) except [Apple -> Google, Google -> Apple]"
        ],
        held(&after)
    );
    reverts_to(&s, &after);
    let s = applies(
        &oauth(),
        "module auth\ngrant retag identities.OAuth -> identities.(OAuth | EmailPass), update identities.OAuth (provider) to ops",
    );
    let s = applies(&s, SPLIT);
    assert_eq!(
        vec![
            "ops retag identities (Apple | Google) -> identities (Apple | EmailPass | Google)",
            "ops update identities (Apple | Google) listed []",
        ],
        held(&s)
    );
}

#[test]
fn a_split_excepts_retags_within_a_set_that_held_only_its_source() {
    let s = applies(
        &oauth(),
        "module auth\ngrant retag identities.Federated -> identities.Federated to ops",
    );
    let after = applies(&s, SPLIT);
    assert_eq!(
        vec![
            "ops retag identities set Federated -> identities set Federated except [Apple -> Google, Google -> Apple]"
        ],
        held(&after)
    );
    reverts_to(&s, &after);
    let s = applies(
        &oauth(),
        "module auth\ngrant retag identities.Federated -> identities.Federated, update identities.Federated to ops",
    );
    let s = applies(&s, SPLIT);
    assert_eq!(
        vec![
            "ops retag identities set Federated -> identities set Federated",
            "ops update identities set Federated frozen [Apple.sub, Google.gid, id, user]",
        ],
        held(&s)
    );
}

#[test]
fn an_unnest_leaves_out_retags_between_its_products_without_update_of_the_nesting_field() {
    let s = applies(&grants(), NEST);
    let s = applies(
        &s,
        "module auth\ngrant retag identities.Social -> identities.(Social | EmailPass) to ops",
    );
    let after = applies(&s, "module auth\nunnest constructor Identity.Social");
    assert_eq!(
        vec![
            "ops retag identities (Apple | Google) -> identities (Apple | EmailPass | Google) except [Apple -> Google, Google -> Apple]"
        ],
        held(&after)
    );
    reverts_to(&s, &after);
    let s = applies(&grants(), NEST);
    let s = applies(
        &s,
        "module auth\ngrant retag identities.Social -> identities.(Social | EmailPass), update identities.Social (login) to ops",
    );
    let s = applies(&s, "module auth\nunnest constructor Identity.Social");
    assert_eq!(
        vec![
            "ops retag identities (Apple | Google) -> identities (Apple | EmailPass | Google)",
            "ops update identities (Apple | Google) listed [Apple.sub, Google.gid]",
        ],
        held(&s)
    );
}

#[test]
fn a_refusal_is_stated_once_for_each_role_relation_privilege_and_product() {
    let s = applies(
        &grants(),
        "module auth\ngrant retag identities.Google -> identities.Apple, read identities.Google, read identities.Google (gid) to support",
    );
    let Err(lines) = migrate(&s, MERGE) else {
        panic!("the merge applies");
    };
    assert_eq!(
        vec![
            "the merge of `Google`, `Apple` into `Social` cannot carry the `retag` grant of `support` on `identities`, which does not cover each of them alike; grant `retag` on `identities.Social` to `support` in the same migration",
            "the merge of `Google`, `Apple` into `Social` cannot carry the `read` grant of `support` on `identities`, which does not cover each of them alike; grant `read` on `identities.Social` to `support` in the same migration",
        ],
        lines
    );
}

const REGION: &str = r"
module auth
type User = { id: Key users, name: Text }
type Provider = Google | Apple
type Identity { id: Key identities, user: Key users, region: Text }
  = EmailPass { email: Text, hash: Bytes }
  | OAuth     { provider: Provider, sub: Text }
relation users : User key id generate sequence
relation identities : Identity key id generate uuidv7
set Federated of identities = OAuth
";

#[test]
fn a_split_routed_by_a_guard_on_a_header_field_excepts_retags_without_update_of_it() {
    let guard = "module auth
split constructor Identity.OAuth into Eu { sub: Text } | Us { sub: Text }
  by match
    | OAuth o if o.region == \"eu\" -> Eu { sub: o.sub }
    | OAuth o -> Us { sub: o.sub }";
    let s = applies(
        &source(REGION),
        "module auth\ngrant retag identities -> identities, update identities.OAuth (provider, sub) to ops",
    );
    let s = applies(&s, guard);
    assert_eq!(
        vec![
            "ops retag identities * -> identities * except [Eu -> Us, Us -> Eu]",
            "ops update identities (Eu | Us) listed [Eu.sub, Us.sub]",
        ],
        held(&s)
    );
}

#[test]
fn a_split_routed_by_a_header_pattern_excepts_retags_unless_update_of_it_is_held() {
    let pattern = "module auth
split constructor Identity.OAuth into Eu { sub: Text } | Us { sub: Text }
  by match
    | OAuth { region: \"eu\", sub } -> Eu { sub: sub }
    | OAuth { sub } -> Us { sub: sub }";
    let s = applies(
        &source(REGION),
        "module auth\ngrant retag identities -> identities, update identities.OAuth (provider, sub) to ops",
    );
    let s = applies(&s, pattern);
    assert_eq!(
        vec![
            "ops retag identities * -> identities * except [Eu -> Us, Us -> Eu]",
            "ops update identities (Eu | Us) listed [Eu.sub, Us.sub]",
        ],
        held(&s)
    );
    let s = applies(
        &source(REGION),
        "module auth\ngrant retag identities -> identities, update identities.OAuth (region) to ops",
    );
    let s = applies(&s, pattern);
    assert_eq!(
        vec![
            "ops retag identities * -> identities *",
            "ops update identities (Eu | Us) listed [region]",
        ],
        held(&s)
    );
}

#[test]
fn an_excepting_retag_is_revoked_by_its_written_text() {
    for (grant, revoke) in [
        (
            "grant retag identities -> identities to ops",
            "revoke retag identities -> identities from ops",
        ),
        (
            "grant retag identities.Federated -> identities to ops",
            "revoke retag identities.Federated -> identities from ops",
        ),
    ] {
        let s = applies(&oauth(), &format!("module auth\n{grant}"));
        let s = applies(&s, SPLIT);
        assert_eq!(1, held(&s).len(), "{grant}");
        assert!(
            held(&s)[0].ends_with("except [Apple -> Google, Google -> Apple]"),
            "{grant}"
        );
        let s = applies(&s, &format!("module auth\n{revoke}"));
        assert_eq!(Vec::<String>::new(), held(&s), "{grant}");
    }
}

#[test]
fn an_excepting_retag_covers_a_constructor_added_later() {
    let s = applies(
        &oauth(),
        "module auth\ngrant retag identities -> identities, retag identities.Federated -> identities to ops",
    );
    let s = applies(&s, SPLIT);
    let s = applies(
        &s,
        "module auth\nadd constructor Identity.Github { login: Text }\n  joins identities.Federated",
    );
    assert_eq!(
        vec![
            "ops retag identities * -> identities * except [Apple -> Google, Google -> Apple]",
            "ops retag identities set Federated -> identities * except [Apple -> Google, Google -> Apple]",
        ],
        held(&s)
    );
    assert_eq!(
        "set Federated of identities = Google | Apple | Github",
        flat(&decl(&s, "set", "identities.Federated"))
    );
}

#[test]
fn a_split_gives_read_of_a_product_without_the_routed_field() {
    let s = applies(
        &oauth(),
        "module auth\ngrant read identities.OAuth (user, sub) to web",
    );
    let s = applies(&s, SPLIT);
    assert_eq!(
        vec!["web read identities (Apple | Google) listed [Apple.sub, Google.gid, user]"],
        held(&s)
    );
}

const WORK: &str = r"
module auth
type User = { id: Key users, name: Text }
type Provider = Google | Apple
type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes }
  | OAuth     { provider: Provider, sub: Text, work: Bool }
relation users : User key id generate sequence
relation identities : Identity key id generate uuidv7
";

const SPLIT_WORK: &str = "module auth
split constructor Identity.OAuth into Google { gid: Text, work: Bool } | Apple { sub: Text, work: Bool }
  by match
    | OAuth { provider: Google, sub, work } -> Google { gid: sub, work: work }
    | OAuth { provider: Apple, sub, work }  -> Apple { sub: sub, work: work }";

const SPLIT_GOOGLE: &str = "module auth
split constructor Identity.Google into Gw { gid: Text } | Gp { gid: Text }
  by match
    | Google { gid, work: true }  -> Gw { gid: gid }
    | Google { gid, work: false } -> Gp { gid: gid }";

const RESPLIT: &str = "ops retag identities * -> identities * except [Apple -> Gp, Apple -> Gw, Gp -> Apple, Gp -> Gw, Gw -> Apple, Gw -> Gp]";

#[test]
fn a_later_split_carries_the_excepted_pairs_to_its_products() {
    let s = applies(
        &source(WORK),
        "module auth\ngrant retag identities -> identities to ops",
    );
    let first = applies(&s, SPLIT_WORK);
    assert_eq!(
        vec!["ops retag identities * -> identities * except [Apple -> Google, Google -> Apple]"],
        held(&first)
    );
    let second = applies(&first, SPLIT_GOOGLE);
    assert_eq!(vec![RESPLIT], held(&second));
    reverts_to(&first, &second);
}

#[test]
fn two_splits_in_one_migration_carry_the_excepted_pairs() {
    let s = applies(
        &source(WORK),
        "module auth\ngrant retag identities -> identities to ops",
    );
    let after = applies(
        &s,
        &format!(
            "{SPLIT_WORK}\n{}",
            SPLIT_GOOGLE.trim_start_matches("module auth\n")
        ),
    );
    assert_eq!(vec![RESPLIT], held(&after));
    reverts_to(&s, &after);
}

const LOCAL: &str = r"
module auth
type User = { id: Key users, name: Text }
type Provider = Google | Apple
type Src = Google | Local
type L = Google { gid: Text } | Local { gid: Text }
type Identity { id: Key identities, user: Key users }
  = Local { gid: Text }
  | OAuth { provider: Provider, sub: Text }
relation users : User key id generate sequence
relation identities : Identity key id generate uuidv7
";

const MERGE_LOCAL: &str = "module auth
merge constructor Identity.(Google | Local) into M { src: Src, gid: Text }
  by match
    | Google g -> M { src: Google, gid: g.gid }
    | Local l  -> M { src: Local, gid: l.gid }";

const NEST_LOCAL: &str =
    "module auth\nnest constructor Identity.(Google | Local) into X { login: L }";

fn split_local() -> Source {
    let s = applies(
        &source(LOCAL),
        "module auth\ngrant retag identities -> identities to ops",
    );
    let s = applies(&s, SPLIT);
    assert_eq!(
        vec!["ops retag identities * -> identities * except [Apple -> Google, Google -> Apple]"],
        held(&s)
    );
    s
}

#[test]
fn a_merge_of_sources_the_excepted_pairs_tell_apart_is_refused_unless_restated() {
    let s = split_local();
    refuses(
        &s,
        MERGE_LOCAL,
        "the merge of `Local`, `Google` into `M` cannot carry the `retag` grant of `ops` on `identities`",
    );
    let after = applies(
        &s,
        &format!("{MERGE_LOCAL}\ngrant retag identities.M -> identities to ops"),
    );
    assert_eq!(
        vec![
            "ops retag identities * -> identities * except [Apple -> M, M -> Apple]",
            "ops retag identities (M) -> identities *",
        ],
        held(&after)
    );
    reverts_to(&s, &after);
}

#[test]
fn a_nest_of_sources_the_excepted_pairs_tell_apart_is_refused_unless_restated() {
    let s = split_local();
    refuses(
        &s,
        NEST_LOCAL,
        "the nest of `Local`, `Google` into `X` cannot carry the `retag` grant of `ops` on `identities`",
    );
    let after = applies(
        &s,
        &format!("{NEST_LOCAL}\ngrant retag identities.X -> identities to ops"),
    );
    assert_eq!(
        vec![
            "ops retag identities * -> identities * except [Apple -> X, X -> Apple]",
            "ops retag identities (X) -> identities *",
        ],
        held(&after)
    );
    reverts_to(&s, &after);
}

#[test]
fn a_merge_of_products_excepted_from_each_other_withholds_update_of_the_discriminator() {
    let s = applies(
        &oauth(),
        "module auth\ngrant retag identities -> identities, update identities.OAuth (sub) to ops",
    );
    let s = applies(&s, SPLIT);
    let s = applies(
        &s,
        "module auth
merge constructor Identity.(Google | Apple) into Social
  { provider: Provider, sub: Text }
  by match
    | Google g -> Social { provider: Google, sub: g.gid }
    | Apple a  -> Social { provider: Apple, sub: a.sub }",
    );
    assert_eq!(
        vec![
            "ops retag identities * -> identities *",
            "ops update identities (Social) listed [Social.sub]",
        ],
        held(&s)
    );
}

/// Returns the requirements of ad hoc `text` over `src` that `role` lacks with grant option,
/// each as its text and privilege.
fn lacking(src: &Source, role: &str, text: &str) -> Vec<(String, &'static str)> {
    use crate::permission::{EffectiveGrants, Mode};
    let (schema, program) = src.check().expect("the source checks");
    let items = crate::parse::parse_adhoc(text, Options::default())
        .ok()
        .expect("the text parses");
    let checked = check::check_adhoc(&schema, &program, &items, &BTreeMap::new())
        .unwrap_or_else(|e| panic!("{text}: {e:?}"));
    let needs = crate::permission::adhoc(&schema, &program, &checked)
        .expect("requirements analyze")
        .remove(0);
    let held = EffectiveGrants::of(&schema, &src.grants, &[], role);
    needs
        .missing(&held, Mode::Registration)
        .iter()
        .map(|r| r.text(&schema))
        .collect()
}

#[test]
fn effective_grants_cover_constructors_a_whole_or_set_grant_gains_later() {
    let s = applies(
        &grants(),
        "module auth\ngrant read identities to web with grant option\ngrant read identities.OAuth to ops with grant option",
    );
    let s = applies(
        &s,
        "module auth\nadd constructor Identity.Github { login: Text }\n  joins identities.OAuth",
    );
    let read = "from i in identities.Github select i.login";
    assert_eq!(Vec::<(String, &str)>::new(), lacking(&s, "web", read));
    assert_eq!(Vec::<(String, &str)>::new(), lacking(&s, "ops", read));
    assert_eq!(
        vec![
            ("reads identities.EmailPass".to_owned(), "read"),
            ("reads identities.EmailPass (id)".to_owned(), "read"),
            ("reads identities.EmailPass (hash)".to_owned(), "read"),
        ],
        lacking(&s, "ops", "from i in identities.EmailPass select i.hash")
    );
}

#[test]
fn a_grant_without_grant_option_or_of_another_role_covers_nothing_at_registration() {
    let s = applies(
        &grants(),
        "module auth\ngrant read identities.EmailPass (email) to web",
    );
    let text = "from i in identities.EmailPass select i.email";
    let want = vec![
        ("reads identities.EmailPass".to_owned(), "read"),
        ("reads identities.EmailPass (id)".to_owned(), "read"),
        ("reads identities.EmailPass (email)".to_owned(), "read"),
    ];
    assert_eq!(want, lacking(&s, "web", text));
    assert_eq!(want, lacking(&s, "ops", text));
    let s = applies(
        &s,
        "module auth\nrevoke read identities.EmailPass (email) from web\ngrant read identities.EmailPass (email) to web with grant option",
    );
    assert_eq!(Vec::<(String, &str)>::new(), lacking(&s, "web", text));
}

#[test]
fn a_dormant_grant_covers_nothing() {
    let mut s = applies(
        &grants(),
        "module auth\ngrant read identities to web with grant option",
    );
    for g in &mut s.grants {
        g.dormant = true;
    }
    assert_eq!(
        vec![
            ("reads identities.EmailPass".to_owned(), "read"),
            ("reads identities.EmailPass (id)".to_owned(), "read"),
            ("reads identities.EmailPass (email)".to_owned(), "read"),
        ],
        lacking(&s, "web", "from i in identities.EmailPass select i.email")
    );
}

#[test]
fn writes_and_retags_name_their_constructors_and_privileges() {
    let s = applies(
        &grants(),
        "module auth\ngrant read identities to web with grant option",
    );
    let got = lacking(
        &s,
        "web",
        "update i in identities.Google set Apple { sub: i.gid }",
    );
    assert!(
        got.contains(&(
            "retags identities.Google to identities.Apple".to_owned(),
            "retag"
        )),
        "{got:?}"
    );
}

#[test]
fn a_grant_target_covers_only_its_relations_constructors_in_the_schema_it_is_read_against() {
    let before = grants();
    let id = |p: &str| before.ids[p];
    let (identities, users) = (id("relation auth::identities"), id("relation auth::users"));
    let (email, google, apple) = (
        id("ctor auth::Identity.EmailPass"),
        id("ctor auth::Identity.Google"),
        id("ctor auth::Identity.Apple"),
    );
    let oauth = id("set auth::identities.OAuth");
    let user = id("ctor auth::User.User");
    let target = |relation, sel| GrantTarget { relation, sel };
    let schema = before.schema().expect("the schema");
    let cases = [
        (
            target(identities, GrantSel::Whole),
            vec![email, google, apple],
        ),
        (target(users, GrantSel::Whole), vec![user]),
        (
            target(identities, GrantSel::Ctors(vec![google])),
            vec![google],
        ),
        (
            target(identities, GrantSel::Ctors(vec![google, apple])),
            vec![google, apple],
        ),
        (target(identities, GrantSel::Ctors(vec![99_999])), vec![]),
        (target(identities, GrantSel::Ctors(vec![user])), vec![]),
        (
            target(identities, GrantSel::Set(oauth)),
            vec![google, apple],
        ),
        (target(users, GrantSel::Set(oauth)), vec![]),
        (target(identities, GrantSel::Set(99_998)), vec![]),
        (target(99_997, GrantSel::Whole), vec![]),
    ];
    for (t, want) in cases {
        let want: BTreeSet<u64> = want.into_iter().collect();
        assert_eq!(want, t.ctors(&schema), "{t:?}");
    }
    let after = applies(&before, MERGE);
    let schema = after.schema().expect("the schema");
    let social = after.ids["ctor auth::Identity.Social"];
    let merged = [
        (target(identities, GrantSel::Whole), vec![email, social]),
        (
            target(identities, GrantSel::Ctors(vec![google, apple])),
            vec![],
        ),
        (target(identities, GrantSel::Set(oauth)), vec![social]),
    ];
    for (t, want) in merged {
        let want: BTreeSet<u64> = want.into_iter().collect();
        assert_eq!(want, t.ctors(&schema), "after the merge: {t:?}");
    }
}

/// Returns the target text of each log grant of `src`.
fn rows(src: &Source) -> Vec<String> {
    let schema = src.schema().expect("the schema builds");
    src.grants
        .iter()
        .map(|g| {
            g.privilege
                .target_text(&schema)
                .expect("a grant names live objects")
        })
        .collect()
}

#[test]
fn each_target_form_is_granted_reprinted_in_id_order_and_revoked_by_its_text() {
    for (privilege, written, row) in [
        ("read", "identities", None),
        (
            "read",
            "identities.(Google | EmailPass) (Google.gid, user, EmailPass.email)",
            Some("auth::identities.(EmailPass | Google) (user, EmailPass.email, Google.gid)"),
        ),
        ("read", "identities.OAuth", None),
        (
            "read",
            "identities.Apple frozen (sub, id)",
            Some("auth::identities.Apple frozen (id, Apple.sub)"),
        ),
        (
            "update",
            "identities.(Apple | Google) (user)",
            Some("auth::identities.(Google | Apple) (user)"),
        ),
        (
            "insert",
            "identities.(Apple | EmailPass)",
            Some("auth::identities.(EmailPass | Apple)"),
        ),
        ("delete", "identities.Apple", Some("auth::identities.Apple")),
        (
            "retag",
            "identities -> identities except (Google -> Apple, Apple -> Google, Apple -> EmailPass)",
            Some(
                "auth::identities -> auth::identities except (Google -> Apple, Apple -> EmailPass, Apple -> Google)",
            ),
        ),
    ] {
        let s = applies(
            &grants(),
            &format!("module auth\ngrant {privilege} {written} to support"),
        );
        let got = rows(&s);
        assert_eq!(1, got.len(), "{written}");
        if let Some(row) = row {
            assert_eq!(row, got[0], "{written}");
        }
        let schema = s.schema().unwrap();
        assert_eq!(
            Some(&s.grants[0].privilege),
            Granted::read_target(&schema, privilege, &got[0]).as_ref(),
            "{written}"
        );
        let regrant = applies(
            &s,
            &format!("module auth\ngrant {privilege} {} to support", got[0]),
        );
        assert_eq!(got, rows(&regrant), "{written}");
        let gone = applies(
            &s,
            &format!("module auth\nrevoke {privilege} {} from support", got[0]),
        );
        assert_eq!(Vec::<String>::new(), rows(&gone), "{written}");
    }
}

#[test]
fn a_function_grant_is_revoked_by_its_text() {
    let s = applies(
        &grants(),
        "module auth\ndefine fn shown (n: Text) -> Text = n\n",
    );
    let s = applies(&s, "module auth\ngrant call fn shown to support");
    assert_eq!(vec!["auth::shown"], rows(&s));
    let s = applies(&s, "module auth\nrevoke call fn auth::shown from support");
    assert_eq!(Vec::<String>::new(), rows(&s));
}

#[test]
fn a_revoke_resolves_to_row_form_before_it_compares() {
    let s = applies(
        &grants(),
        "module auth\ngrant read identities.EmailPass (user, email) to support",
    );
    assert_eq!(
        vec!["auth::identities.EmailPass (user, EmailPass.email)"],
        rows(&s)
    );
    let s = applies(
        &s,
        "module auth\nrevoke read auth::identities.EmailPass (EmailPass.email, user) from support",
    );
    assert_eq!(Vec::<String>::new(), rows(&s));
}

#[test]
fn a_field_named_alone_is_a_header_field_or_one_every_constructor_declares() {
    assert_eq!(
        vec![
            "`gid` is not a field every constructor of the set declares; write it as `C.gid`"
                .to_owned()
        ],
        migrate(
            &grants(),
            "module auth\ngrant read identities.(Google | Apple) (gid) to support"
        )
        .unwrap_err(),
    );
    assert_eq!(
        vec!["`Apple` is not a constructor this grant covers".to_owned()],
        migrate(
            &grants(),
            "module auth\ngrant read identities.Google (Apple.sub) to support"
        )
        .unwrap_err(),
    );
}

#[test]
fn a_revoke_without_a_field_list_takes_every_frozen_set_but_no_listed_one() {
    let s = applies(
        &grants(),
        "module auth\ngrant read identities.Google to support\ngrant read identities.Google (user) to support",
    );
    let s = applies(
        &s,
        "module auth\nadd field Identity.Google.verified: Bool fill false",
    );
    let s = applies(&s, "module auth\ngrant read identities.Google to support");
    assert_eq!(3, rows(&s).len());
    let s = applies(
        &s,
        "module auth\nrevoke read identities.Google from support",
    );
    assert_eq!(vec!["auth::identities.Google (user)"], rows(&s));
}

#[test]
fn a_frozen_revoke_takes_only_its_own_field_set() {
    let s = applies(
        &grants(),
        "module auth\ngrant read identities.Google frozen (id) to support\ngrant read identities.Google frozen (id, user) to support",
    );
    let s = applies(
        &s,
        "module auth\nrevoke read identities.Google frozen (id) from support",
    );
    assert_eq!(vec!["auth::identities.Google frozen (id, user)"], rows(&s));
}

#[test]
fn a_retag_revoke_with_except_takes_only_those_pairs() {
    let s = applies(
        &grants(),
        "module auth\ngrant retag identities -> identities except (Google -> Apple) to ops",
    );
    assert_eq!(
        vec![
            "`revoke retag identities -> identities except (Apple -> Google) from ops` matches no grant `ops` holds"
                .to_owned()
        ],
        migrate(
            &s,
            "module auth\nrevoke retag identities -> identities except (Apple -> Google) from ops"
        )
        .unwrap_err(),
    );
    let gone = applies(
        &s,
        "module auth\nrevoke retag identities -> identities except (Google -> Apple) from ops",
    );
    assert_eq!(Vec::<String>::new(), rows(&gone));
    let widened = applies(
        &s,
        "module auth\nrevoke retag identities -> identities from ops",
    );
    assert_eq!(Vec::<String>::new(), rows(&widened));
}

#[test]
fn an_empty_revoke_refuses_the_migration_naming_it_in_every_form() {
    let g = "module auth\ngrant read identities.Google (user), insert identities.Google, retag identities.Google -> identities.Apple to support";
    let s = applies(&grants(), g);
    for revoke in [
        "revoke read identities.Google (id) from support",
        "revoke read identities.Google frozen (id) from support",
        "revoke read identities.Google from support",
        "revoke read identities.Google (user) from web",
        "revoke insert identities.Apple from support",
        "revoke delete identities.Google from support",
        "revoke retag identities.Apple -> identities.Google from support",
        "revoke retag identities.Google -> identities.Apple except (Google -> Apple) from support",
        "revoke insert identities.Google, delete identities.Google from support",
    ] {
        let refused = migrate(&s, &format!("module auth\n{revoke}")).unwrap_err();
        assert_eq!(
            vec![format!(
                "`{revoke}` matches no grant `{}` holds",
                revoke.rsplit(' ').next().unwrap()
            )],
            refused,
        );
    }
}

#[test]
fn a_dormant_grant_is_revoked_by_its_text() {
    let s = applies(
        &grants(),
        "module auth\ngrant read identities.Google (user) to support",
    );
    let mut s = s;
    s.grants[0].dormant = true;
    let s = applies(
        &s,
        "module auth\nrevoke read identities.Google (user) from support",
    );
    assert_eq!(Vec::<String>::new(), rows(&s));
}

#[test]
fn a_carried_grant_is_revoked_by_the_text_it_prints() {
    let s = applies(
        &oauth(),
        "module auth\ngrant read identities.OAuth to support",
    );
    let s = applies(&s, SPLIT);
    let printed = rows(&s);
    assert_eq!(1, printed.len());
    let s = applies(
        &s,
        &format!("module auth\nrevoke read {} from support", printed[0]),
    );
    assert_eq!(Vec::<String>::new(), rows(&s));
}

#[test]
fn a_target_covers_its_queries_and_the_same_or_fewer_constructors_and_fields() {
    let s = grants();
    let schema = s.schema().unwrap();
    let covers = |p, a, b| target_covers(&schema, &NoCalls, p, Some(a), Some(b));
    assert!(covers("call", "web-2.4", "web-2.4::q"));
    assert!(!covers("call", "web-2.4", "web-2.5::q"));
    assert!(!covers(
        "call",
        "web::q at 1 (x: I64) -> Text",
        "web::q at 2 (x: Text) -> Text"
    ));
    assert!(!covers(
        "call",
        "web::q at 1 () -> Text",
        "web::r at 1 () -> Text"
    ));
    assert!(covers(
        "read",
        "auth::identities frozen (id, user, Google.gid)",
        "auth::identities.Google (user, Google.gid)"
    ));
    assert!(!covers(
        "read",
        "auth::identities.Google (user)",
        "auth::identities.Google (user, Google.gid)"
    ));
    assert!(!covers(
        "insert",
        "auth::identities.Google",
        "auth::identities.(Google | Apple)"
    ));
    assert!(target_covers(&schema, &NoCalls, "monitor", None, None));
    assert!(!target_covers(
        &schema,
        &NoCalls,
        "call",
        None,
        Some("web-2.4")
    ));
}

#[test]
fn a_family_revoke_without_a_signature_matches_whatever_signature_the_row_records() {
    let schema = grants().schema().unwrap();
    let matches = |text: &str, row: &str| {
        let items = crate::parse::parse_adhoc(text, Options::default())
            .ok()
            .expect("the revoke parses");
        let ast::AdhocItem::Revoke(r) = &items[0] else {
            panic!("a revoke");
        };
        deployment_revoke(&schema, text, &r.privileges[0])
            .expect("a deployment privilege")
            .matches(&schema, &NoCalls, "call", Some(row))
    };
    let row = "web::login_key at 41 (u: Key auth::users) -> Text";
    assert!(matches("revoke call web::login_key from app", row));
    assert!(!matches("revoke call web::other from app", row));
    assert!(!matches("revoke call web::login from app", row));
    assert!(matches(
        "revoke call web::login_key at 41 (u:  Key auth::users) -> Text from app",
        row
    ));
    assert!(!matches(
        "revoke call web::login_key at 42 (u: Key auth::users) -> Text from app",
        row
    ));
    assert!(matches("revoke call web-2.4 from app", "web-2.4"));
    assert!(!matches("revoke call web-2.4 from app", "web-2.4::q"));
    assert!(matches("revoke call web-2.4::q from app", "web-2.4::q"));
}

#[test]
fn constructors_print_in_id_order_when_a_split_places_later_ids_first() {
    let s = applies(
        &oauth(),
        "module auth\nadd constructor Identity.Later { at: Text }",
    );
    let s = applies(&s, SPLIT);
    let s = applies(
        &s,
        "module auth\ngrant insert identities.(Google | Later) to support",
    );
    assert_eq!(vec!["auth::identities.(Later | Google)"], rows(&s));
    let s = applies(
        &s,
        "module auth\nrevoke insert auth::identities.(Later | Google) from support",
    );
    assert_eq!(Vec::<String>::new(), rows(&s));
}

#[test]
fn a_target_reads_back_with_its_excepted_pairs_sorted_and_each_side_on_its_relation() {
    let schema = grants().schema().unwrap();
    let read = |text| Granted::read_target(&schema, "retag", text).expect("the target reads");
    assert_eq!(
        read("auth::identities -> auth::identities except (Apple -> Google, Google -> Apple)"),
        read("auth::identities -> auth::identities except (Google -> Apple, Apple -> Google)"),
    );
    assert!(
        Granted::read_target(
            &schema,
            "retag",
            "auth::identities -> auth::users except (Google -> User)"
        )
        .is_some()
    );
}

#[test]
fn an_update_revoke_without_a_field_list_takes_every_frozen_set() {
    let s = applies(
        &grants(),
        "module auth\ngrant update identities.Google frozen (id) to support\ngrant update identities.Google frozen (id, user) to support",
    );
    let s = applies(
        &s,
        "module auth\nrevoke update identities.Google from support",
    );
    assert_eq!(Vec::<String>::new(), rows(&s));
}

#[test]
fn a_retag_target_covers_a_retag_between_fewer_constructors() {
    let schema = grants().schema().unwrap();
    let covers = |a, b| target_covers(&schema, &NoCalls, "retag", Some(a), Some(b));
    let whole = "auth::identities -> auth::identities";
    let part = "auth::identities.Google -> auth::identities.Apple";
    assert!(covers(whole, part));
    assert!(!covers(part, whole));
}

#[test]
fn an_option_closes_a_cycle_when_its_grantee_reaches_its_grantor() {
    let schema = grants().schema().unwrap();
    let edge = |grantor, grantee, target| GrantRow {
        grantor,
        grantee,
        privilege: "call",
        target,
        grant_option: true,
        dormant: false,
    };
    let closes = |rows: &[GrantRow<'_>], new| closes_option_cycle(&schema, &NoCalls, rows, new);
    let held = edge("a", "b", Some("web-2.4"));
    assert!(closes(&[], edge("a", "a", None)));
    assert!(closes(&[held], edge("b", "a", Some("web-2.4"))));
    assert!(closes(&[held], edge("b", "a", Some("web-2.4::q"))));
    assert!(!closes(&[held], edge("b", "a", Some("web-2.5"))));
    assert!(!closes(&[held], edge("a", "c", Some("web-2.4"))));
    let no_option = GrantRow {
        grant_option: false,
        ..held
    };
    assert!(!closes(&[no_option], edge("b", "a", Some("web-2.4"))));
    let dormant = GrantRow {
        dormant: true,
        ..held
    };
    assert!(closes(&[dormant], edge("b", "a", Some("web-2.4"))));
}

fn monitor_row<'a>(grantor: &'a str, grantee: &'a str) -> GrantRow<'a> {
    GrantRow {
        grantor,
        grantee,
        privilege: "monitor",
        target: None,
        grant_option: true,
        dormant: false,
    }
}

#[test]
fn an_option_supports_a_row_its_grantee_gave_of_the_same_privilege_on_a_covered_target() {
    let schema = grants().schema().unwrap();
    let option = monitor_row("owner", "ann");
    let given = GrantRow {
        grant_option: false,
        ..monitor_row("ann", "bob")
    };
    assert!(option.supports(&schema, &NoCalls, &given));
    assert!(option.keeps(&schema, &NoCalls, &given));
    let unheld = GrantRow {
        grant_option: false,
        ..option
    };
    assert!(!unheld.supports(&schema, &NoCalls, &given));
    assert!(!unheld.keeps(&schema, &NoCalls, &given));
    let other_grantor = monitor_row("cal", "bob");
    assert!(!option.supports(&schema, &NoCalls, &other_grantor));
    let other_privilege = GrantRow {
        privilege: "cancel",
        ..given
    };
    assert!(!option.supports(&schema, &NoCalls, &other_privilege));
    let other_target = GrantRow {
        target: Some("web-2.4"),
        ..given
    };
    assert!(!option.supports(&schema, &NoCalls, &other_target));
}

#[test]
fn a_dormant_option_supports_a_row_but_does_not_keep_it() {
    let schema = grants().schema().unwrap();
    let given = monitor_row("ann", "bob");
    let dormant = GrantRow {
        dormant: true,
        ..monitor_row("owner", "ann")
    };
    assert!(dormant.supports(&schema, &NoCalls, &given));
    assert!(!dormant.keeps(&schema, &NoCalls, &given));
}

#[test]
fn a_role_s_own_option_does_not_keep_a_row_it_gave() {
    let schema = grants().schema().unwrap();
    let given = monitor_row("ann", "bob");
    let own = monitor_row("ann", "ann");
    assert!(own.supports(&schema, &NoCalls, &given));
    assert!(!own.keeps(&schema, &NoCalls, &given));
}

#[test]
fn the_schema_owner_s_option_supports_nothing_that_can_be_orphaned() {
    let schema = grants().schema().unwrap();
    let option = monitor_row("web", "owner");
    let given = monitor_row("owner", "bob");
    assert!(!option.supports(&schema, &NoCalls, &given));
    assert!(option.keeps(&schema, &NoCalls, &given));
}

#[test]
fn a_retag_revoke_takes_its_except_pairs_in_any_order() {
    let s = applies(
        &grants(),
        "module auth\ngrant retag identities -> identities except (Google -> Apple, Apple -> Google) to ops",
    );
    let s = applies(
        &s,
        "module auth\nrevoke retag identities -> identities except (Apple -> Google, Google -> Apple) from ops",
    );
    assert_eq!(Vec::<String>::new(), rows(&s));
}

/// Signatures read by the schema at the version each target writes, and one bundle's query.
struct FamilyAt(Vec<(u64, Schema)>, Ids);

impl CallSignatures for FamilyAt {
    fn declares(&self, _: &str, _: &str) -> Option<bool> {
        None
    }

    fn family(&self, target: &str) -> Option<Ids> {
        let s = crate::signature::family_signature(target)?;
        let (_, schema) = self.0.iter().find(|(v, _)| *v == s.at)?;
        crate::signature::signature_ids(schema, &s).ok()
    }

    fn query(&self, bundle: &str, query: &str) -> Option<Ids> {
        (bundle == "web-2.4" && query == "q").then(|| self.1.clone())
    }
}

const SIG_SHOP: &str = "module shop\nadd module shop\nadd type Kind = A | B { n: I64 }\nadd type Item = { id: Key items, kind: Kind }\nadd relation items : Item key id\n";

fn sig_source(ops: &[&str]) -> Source {
    let mut s = Source::default();
    for text in ops {
        let file = parse_migration(text, Options::default())
            .ok()
            .expect("the migration parses");
        s = s.migrate(&file, text).expect("the migration applies");
    }
    s
}

fn ids_at(schema: &Schema, target: &str) -> Ids {
    let s = crate::signature::family_signature(target).expect("a family target");
    crate::signature::signature_ids(schema, &s).expect("the signature reads")
}

#[test]
fn a_printed_family_target_reads_back_as_the_same_ids() {
    let schema = sig_source(&[SIG_SHOP]).schema().unwrap();
    let item = schema.module(schema.module_named("shop").unwrap()).types["Item"];
    let rel = schema
        .relations
        .iter()
        .position(|r| r.name == "items")
        .unwrap();
    let key = Ty::Key {
        rel: RelId(rel as u32),
        ctors: None,
    };
    let result = Ty::Set(Box::new(Ty::Record(vec![
        (
            "i".to_owned(),
            Ty::Data {
                def: item,
                args: Vec::new(),
                ctors: None,
            },
        ),
        ("k".to_owned(), key.clone()),
    ])));
    let params = vec![("u".to_owned(), key.clone())];
    let target = crate::signature::family_target(&schema, "web", "q", 1, &params, &result);
    assert_eq!(
        "web::q at 1 (u: Key shop::items) -> Set { i: shop::Item, k: Key shop::items }",
        target
    );
    assert_eq!(Ids::of(&schema, &[key], &result), ids_at(&schema, &target));
}

#[test]
fn a_rename_keeps_the_ids_and_a_constructor_or_field_change_does_not() {
    let source = sig_source(&[SIG_SHOP]);
    let target = "web::q at 1 () -> shop::Item";
    let before = ids_at(&source.schema().unwrap(), target);
    let renamed = sig_source(&[SIG_SHOP, "module shop\nrename type Item to Thing\n"]);
    assert_eq!(
        before,
        ids_at(&renamed.schema().unwrap(), "web::q at 2 () -> shop::Thing")
    );
    let widened = sig_source(&[SIG_SHOP, "module shop\nadd constructor Kind.C\n"]);
    assert_ne!(
        before,
        ids_at(&widened.schema().unwrap(), "web::q at 2 () -> shop::Item"),
        "a nested named type's new constructor changes the signature"
    );
    let other = sig_source(&[
        SIG_SHOP,
        "module shop\nadd type Other = { id: I64, kind: Kind }\n",
    ]);
    assert_ne!(
        before,
        ids_at(&other.schema().unwrap(), "web::q at 2 () -> shop::Other"),
    );
}

#[test]
fn a_key_is_its_relation_and_selection_whatever_the_relation_gains() {
    let source = sig_source(&[SIG_SHOP]);
    let target = "web::q at 1 (u: Key shop::items) -> Text";
    let before = ids_at(&source.schema().unwrap(), target);
    let changed = sig_source(&[
        SIG_SHOP,
        "module shop\nadd field Item.note: Text fill \"\"\n",
    ]);
    assert_eq!(
        before,
        ids_at(
            &changed.schema().unwrap(),
            "web::q at 2 (u: Key shop::items) -> Text"
        ),
    );
}

#[test]
fn a_parameter_name_is_not_part_of_the_ids() {
    let schema = sig_source(&[SIG_SHOP]).schema().unwrap();
    assert_eq!(
        ids_at(&schema, "web::q at 1 (a: I64) -> Text"),
        ids_at(&schema, "web::q at 1 (b: I64) -> Text"),
    );
    assert_ne!(
        ids_at(&schema, "web::q at 1 (a: I64) -> Text"),
        ids_at(&schema, "web::q at 1 (a: Text) -> Text"),
    );
    assert_ne!(
        ids_at(&schema, "web::q at 1 () -> { a: I64 }"),
        ids_at(&schema, "web::q at 1 () -> { b: I64 }"),
    );
}

#[test]
fn a_signature_writes_a_schema_type_as_module_and_name() {
    let schema = sig_source(&[SIG_SHOP]).schema().unwrap();
    let s = crate::signature::family_signature("web::q at 1 () -> Item").unwrap();
    assert!(crate::signature::signature_ids(&schema, &s).is_err());
    assert!(crate::signature::family_signature("web::q").is_none());
}

#[test]
fn a_family_target_covers_by_ids_and_counts_an_unread_one_as_supported_only() {
    let schema = sig_source(&[SIG_SHOP]).schema().unwrap();
    let int = Ids::of(&schema, &[Ty::Base(Base::I64)], &Ty::Base(Base::Text));
    let calls = FamilyAt(vec![(1, schema.clone()), (2, schema.clone())], int);
    let family = "web::q at 1 (x: I64) -> Text";
    let covers = |by, row| target_covers(&schema, &calls, "call", Some(by), Some(row));
    assert!(covers(family, "web::q at 2 (y: I64) -> Text"));
    assert!(!covers(family, "web::q at 2 (x: Text) -> Text"));
    assert!(covers(family, "web-2.4::q"));
    assert!(!covers("web::q at 1 (x: Text) -> Text", "web-2.4::q"));
    assert!(!covers(family, "api-2.4::q"));
    assert!(
        !covers(family, "web-2.5::q"),
        "an unread bundle is not covered"
    );

    let row = |grantor, grantee, target| GrantRow {
        grantor,
        grantee,
        privilege: "call",
        target: Some(target),
        grant_option: true,
        dormant: false,
    };
    let holder = row("owner", "ann", family);
    let unread = row("ann", "bob", "web-2.5::q");
    assert!(holder.supports(&schema, &calls, &unread));
    assert!(!holder.keeps(&schema, &calls, &unread));
    assert!(holder.supports(&schema, &calls, &row("ann", "bob", "web-2.4::q")));
    assert!(!holder.supports(&schema, &calls, &row("ann", "bob", "api-2.4::q")));
}

#[test]
fn a_whole_bundle_row_is_a_known_shape_to_a_family_row() {
    let schema = grants().schema().unwrap();
    let row = |grantor, grantee, target| GrantRow {
        grantor,
        grantee,
        privilege: "call",
        target: Some(target),
        grant_option: true,
        dormant: false,
    };
    let family = "web::q at 2 (n: I64) -> Set Text";
    let option = row("owner", "ann", family);
    assert!(!option.supports(&schema, &NoCalls, &row("ann", "cid", "mine-1.0")));
    assert!(!option.supports(&schema, &NoCalls, &row("ann", "cid", "nohyphen::q")));
    assert!(!option.supports(&schema, &NoCalls, &row("ann", "cid", "fn shop::names")));
    assert!(option.supports(&schema, &NoCalls, &row("ann", "cid", "web-1.0::q")));

    let closes = |held, new| closes_option_cycle(&schema, &NoCalls, &[held], new);
    assert!(!closes(
        row("g", "owner", family),
        row("owner", "g", "fn shop::names")
    ));
    assert!(!closes(
        row("g", "owner", family),
        row("owner", "g", "mine-1.0")
    ));
    assert!(closes(
        row("g", "owner", family),
        row("owner", "g", "web-1.0")
    ));
}
