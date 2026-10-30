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
    let held = EffectiveGrants::of(&schema, &src.grants, role);
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
