//! The query examples of the queries part of the spec, evaluated on a small database.

mod common;

use adtb_reference::{Fault, Value};
use common::*;

const SCHEMA: &str = r#"
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
set OAuth of identities = Google | Apple
fn display_name (i: Identity) -> Text = match i
  | EmailPass e -> e.email
  | Google g    -> g.gid
  | Apple a     -> a.sub

type Post = { id: Key posts, author: Key users, created_at: Instant, title: Text }
type Phone = { id: Key phones, owner: Key users on delete cascade, seen: Instant }
type Follow = { src: Key users, dst: Key users }
type Employee = { id: Key employees, name: Text, manager: Option (Key employees) }
type Order = { id: Key orders, user: Key users, total: I64, qty: I64 }
type Reset = { id: Key resets, identity: Key identities.EmailPass on delete cascade }
type Raw = { id: Key raws, n: I64 }
relation posts : Post key id generate sequence
relation phones : Phone key id generate sequence
relation follows : Follow key (src, dst)
relation employees : Employee key id generate sequence
relation orders : Order key id generate sequence
relation resets : Reset key id generate sequence
relation raws : Raw key id generate sequence

type Via = Courier { name: Text } | Mail { tracking: Text }
type Shipment = Pending | Shipped { via: Via }
type Status = Draft | Sent { at: Instant } | Paid { at: Instant }
type Invoice = { id: Key invoices, status: Status, issued: Instant }
relation invoices : Invoice key id generate sequence
type Session = { id: Key sessions, owner: Option (Key users) }
relation sessions : Session key id generate sequence

fn sign_ins () -> Set (Local { id: Key identities, email: Text }
                     | Remote { id: Key identities, provider: Text }) =
  from i in identities select match i
    | EmailPass e -> Local  { id: key e, email: e.email }
    | Google g    -> Remote { id: key g, provider: "google" }
    | Apple a     -> Remote { id: key a, provider: "apple" }

fn logins_of (u: Key users) -> Set Text =
  from i in identities where i.user == u select display_name(i)

fn active () -> Set User = from u in users where exists i in u.<identities.user select u

fn having (p: Identity -> Bool) -> Set (Key users) =
  from i in identities where p(i) select i.user
"#;

fn t(micros: i64) -> Value {
    Value::Instant(micros)
}

fn uuid(n: u8) -> String {
    format!("identities.key(uuid(\"00000000-0000-7000-8000-0000000000{n:02}\"))")
}

fn fixture() -> Fixture {
    let mut f = Fixture::new(SCHEMA);
    f.rows(
        "users",
        r#"[User { id: users.key(1), name: "ann" },
            User { id: users.key(2), name: "bo" },
            User { id: users.key(3), name: "cy" }]"#,
    );
    f.rows(
        "identities",
        &format!(
            r#"[EmailPass {{ id: {}, user: users.key(1), email: "ann@example.com", hash: bytes("h1") }},
                Google {{ id: {}, user: users.key(1), gid: "g-ann" }},
                Google {{ id: {}, user: users.key(2), gid: "root" }},
                Apple {{ id: {}, user: users.key(2), sub: "a-bo" }}]"#,
            uuid(1),
            uuid(2),
            uuid(3),
            uuid(4)
        ),
    );
    let times = [
        ("t1", "Instant", t(10)),
        ("t2", "Instant", t(20)),
        ("t3", "Instant", t(30)),
        ("t4", "Instant", t(40)),
        ("t5", "Instant", t(50)),
    ];
    f.rows_with(
        "posts",
        r#"[Post { id: posts.key(1), author: users.key(1), created_at: t1, title: "hello" },
            Post { id: posts.key(2), author: users.key(1), created_at: t2, title: "again" },
            Post { id: posts.key(3), author: users.key(2), created_at: t3, title: "bo's" },
            Post { id: posts.key(4), author: users.key(1), created_at: t4, title: "fourth" },
            Post { id: posts.key(5), author: users.key(1), created_at: t5, title: "fifth" }]"#,
        &times,
    );
    f.rows_with(
        "phones",
        r#"[Phone { id: phones.key(1), owner: users.key(1), seen: t1 },
            Phone { id: phones.key(2), owner: users.key(1), seen: t2 }]"#,
        &times,
    );
    f.rows(
        "follows",
        r#"[Follow { src: users.key(1), dst: users.key(2) },
            Follow { src: users.key(2), dst: users.key(3) },
            Follow { src: users.key(3), dst: users.key(2) }]"#,
    );
    f.rows(
        "employees",
        r#"[Employee { id: employees.key(1), name: "boss", manager: None },
            Employee { id: employees.key(2), name: "w", manager: Some { value: employees.key(1) } }]"#,
    );
    f.rows(
        "orders",
        r#"[Order { id: orders.key(1), user: users.key(1), total: 100, qty: 2 },
            Order { id: orders.key(2), user: users.key(1), total: 100, qty: 0 },
            Order { id: orders.key(3), user: users.key(2), total: 30, qty: 3 },
            Order { id: orders.key(4), user: users.key(1), total: 50, qty: 5 }]"#,
    );
    f.rows(
        "resets",
        "from e in identities.EmailPass select Reset { id: resets.key(1), identity: key e }",
    );
    f.rows(
        "raws",
        "[Raw { id: raws.key(1), n: 5 }, Raw { id: raws.key(2), n: 3000000000 }]",
    );
    f.rows_with(
        "invoices",
        r#"[Invoice { id: invoices.key(1), status: Draft, issued: t1 },
            Invoice { id: invoices.key(2), status: Sent { at: t1 }, issued: t2 },
            Invoice { id: invoices.key(3), status: Draft, issued: t3 }]"#,
        &times,
    );
    f.rows(
        "sessions",
        r#"[Session { id: sessions.key(1), owner: Some { value: users.key(1) } },
            Session { id: sessions.key(2), owner: None }]"#,
    );
    f
}

fn rec(fs: Vec<(&str, Value)>) -> Value {
    Value::record(fs)
}

fn row(f: &Fixture, text: &str) -> Value {
    f.one(text)
}

fn user(f: &Fixture, id: i64) -> Value {
    row(
        f,
        &format!("from u in users where u.id == users.key({id}) select u"),
    )
}

fn identity(f: &Fixture, n: u8) -> Value {
    row(
        f,
        &format!("from i in identities where i.id == {} select i", uuid(n)),
    )
}

fn post(f: &Fixture, id: i64) -> Value {
    row(
        f,
        &format!("from p in posts where p.id == posts.key({id}) select p"),
    )
}

#[test]
fn a_query_built_sum_selects_by_constructor() {
    let f = fixture();
    assert_eq!(
        texts(["google", "apple"]),
        f.value("from l in sign_ins().Remote select l.provider")
    );
}

#[test]
fn a_generator_depends_on_an_earlier_one() {
    let f = fixture();
    let got =
        f.value("from u in users, i in u.<identities.user select { name: u.name, id: key i }");
    let want = set([
        rec(vec![("name", text("ann")), ("id", f.value(&uuid(1)))]),
        rec(vec![("name", text("ann")), ("id", f.value(&uuid(2)))]),
        rec(vec![("name", text("bo")), ("id", f.value(&uuid(3)))]),
        rec(vec![("name", text("bo")), ("id", f.value(&uuid(4)))]),
    ]);
    assert_eq!(want, got);
}

#[test]
fn projection_merges_equal_values() {
    let f = fixture();
    assert_eq!(keys([1, 2]), f.value("from p in posts select p.author"));
}

#[test]
fn arms_returning_rows_join_at_the_sum() {
    let f = fixture();
    let got = f.value(
        "from i in identities select match i
           | Google g    -> g
           | Apple a     -> a
           | EmailPass e -> e",
    );
    assert_eq!(f.value("from i in identities select i"), got);
    assert_eq!(4, got.as_set().unwrap().len());
}

#[test]
fn a_registered_query_returns_a_result_sum() {
    let f = fixture();
    let got = f
        .call_query(
            "query login_key (u: Key users) -> Set (ByEmail { email: Text } | ByGoogle { gid: Text }) =
               from i in identities.(EmailPass | Google) where i.user == u
               select match i
                 | EmailPass e -> ByEmail { email: e.email }
                 | Google g    -> ByGoogle { gid: g.gid }",
            vec![key(1)],
        )
        .unwrap();
    let want = set([
        Value::Ctor(0, vec![("email".into(), text("ann@example.com"))]),
        Value::Ctor(1, vec![("gid".into(), text("g-ann"))]),
    ]);
    assert_eq!(want, got);
}

#[test]
fn an_adhoc_wildcard_covers_the_remaining_constructors() {
    let f = fixture();
    let q = r#"match i | Google g -> g.gid | _ -> "other""#;
    let at = |n| {
        f.eval_with(q, &[("i", "Identity", identity(&f, n))])
            .unwrap()
    };
    assert_eq!(text("g-ann"), at(2));
    assert_eq!(text("other"), at(1));
    assert_eq!(text("other"), at(4));
}

#[test]
fn a_constructor_binder_reaches_header_fields() {
    let f = fixture();
    let got = f.value(
        "from i in identities select match i
           | EmailPass e -> { owner: e.user, login: e.email }
           | Google g    -> { owner: g.user, login: g.gid }
           | Apple a     -> { owner: a.user, login: a.sub }",
    );
    let want = set([
        rec(vec![("owner", key(1)), ("login", text("ann@example.com"))]),
        rec(vec![("owner", key(1)), ("login", text("g-ann"))]),
        rec(vec![("owner", key(2)), ("login", text("root"))]),
        rec(vec![("owner", key(2)), ("login", text("a-bo"))]),
    ]);
    assert_eq!(want, got);
}

#[test]
fn a_match_over_a_relation_is_the_union_of_its_arms() {
    let f = fixture();
    let whole = "from i in identities where i.user == u
                 select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub";
    let arms = "(from e in identities.EmailPass where e.user == u select e.email)
                union (from g in identities.Google where g.user == u select g.gid)
                union (from a in identities.Apple where a.user == u select a.sub)";
    for u in 1..=3 {
        let p = [("u", "Key users", key(u))];
        assert_eq!(f.eval_with(whole, &p), f.eval_with(arms, &p));
    }
    let p = [("u", "Key users", key(2))];
    assert_eq!(texts(["root", "a-bo"]), f.eval_with(whole, &p).unwrap());
}

#[test]
fn record_patterns_bind_fields() {
    let f = fixture();
    let q = "match i
               | EmailPass { email, hash: _ } -> email
               | Google { gid }               -> gid
               | Apple { sub }                -> sub";
    let at = |n| {
        f.eval_with(q, &[("i", "Identity", identity(&f, n))])
            .unwrap()
    };
    assert_eq!(text("ann@example.com"), at(1));
    assert_eq!(text("root"), at(3));
    assert_eq!(text("a-bo"), at(4));
}

#[test]
fn nested_patterns_reach_inner_constructors() {
    let f = fixture();
    let q = r#"match s
                 | Shipped { via: Courier c } -> c.name
                 | Shipped { via: Mail m }    -> m.tracking
                 | Pending                    -> "pending""#;
    let at = |v: &str| f.eval_with(q, &[("s", "Shipment", f.value(v))]).unwrap();
    assert_eq!(
        text("dhl"),
        at(r#"Shipped { via: Courier { name: "dhl" } }"#)
    );
    assert_eq!(
        text("t-1"),
        at(r#"Shipped { via: Mail { tracking: "t-1" } }"#)
    );
    assert_eq!(text("pending"), at("Pending"));
}

#[test]
fn a_named_set_pattern_covers_its_members() {
    let f = fixture();
    let names = "match i
                   | EmailPass e -> e.email
                   | OAuth o     -> display_name(o)";
    let users = "match i
                   | EmailPass { user } -> user
                   | OAuth { user }     -> user";
    for (n, name, owner) in [(1, "ann@example.com", 1), (2, "g-ann", 1), (4, "a-bo", 2)] {
        let p = [("i", "Identity", identity(&f, n))];
        assert_eq!(text(name), f.eval_with(names, &p).unwrap());
        assert_eq!(key(owner), f.eval_with(users, &p).unwrap());
    }
}

#[test]
fn a_false_guard_falls_through_to_the_next_arm() {
    let f = fixture();
    let q = r#"match i
                 | Google g if g.gid == "root" -> "staff"
                 | Google _                    -> "guest"
                 | EmailPass _                 -> "guest"
                 | Apple _                     -> "guest""#;
    let at = |n| {
        f.eval_with(q, &[("i", "Identity", identity(&f, n))])
            .unwrap()
    };
    assert_eq!(text("staff"), at(3));
    assert_eq!(text("guest"), at(2));
    assert_eq!(text("guest"), at(1));
}

#[test]
fn literal_patterns_compare_scalars() {
    let f = fixture();
    let q = r#"match c | "US" -> 0 | "CA" -> 1 | _ -> 2"#;
    let at = |c: &str| f.eval_with(q, &[("c", "Text", text(c))]).unwrap();
    assert_eq!(int(0), at("US"));
    assert_eq!(int(1), at("CA"));
    assert_eq!(int(2), at("FR"));
}

#[test]
fn a_join_compares_a_reference_with_a_key() {
    let f = fixture();
    assert_eq!(
        texts(["ann", "bo"]),
        f.value("from i in identities, u in users where i.user == key u select u.name")
    );
}

#[test]
fn a_path_is_a_join() {
    let f = fixture();
    let path = f.value("from i in identities select i.user.name");
    let join = f.value("from i in identities, u in users where key u == i.user select u.name");
    assert_eq!(texts(["ann", "bo"]), path);
    assert_eq!(join, path);
}

#[test]
fn a_path_to_one_constructor_needs_one_arm() {
    let f = fixture();
    assert_eq!(
        texts(["ann@example.com"]),
        f.value("from r in resets select match r.identity | EmailPass e -> e.email")
    );
}

#[test]
fn an_optional_reference_is_matched() {
    let f = fixture();
    let got = f.value(
        r#"from e in employees
           select match e.manager
             | None              -> "none"
             | Some { value: m } -> m.name"#,
    );
    assert_eq!(texts(["none", "boss"]), got);
}

#[test]
fn a_backlink_is_the_rows_that_reference() {
    let f = fixture();
    let p = [("u", "User", user(&f, 1))];
    let back = f.eval_with("u.<identities.user", &p).unwrap();
    let from = f
        .eval_with("from i in identities where i.user == key u select i", &p)
        .unwrap();
    assert_eq!(set([identity(&f, 1), identity(&f, 2)]), back);
    assert_eq!(from, back);
}

#[test]
fn a_backlink_counts_per_row() {
    let f = fixture();
    let got =
        f.value("from u in users select { name: u.name, n: count(i for i in u.<identities.user) }");
    let want = set([
        rec(vec![("name", text("ann")), ("n", int(2))]),
        rec(vec![("name", text("bo")), ("n", int(2))]),
        rec(vec![("name", text("cy")), ("n", int(0))]),
    ]);
    assert_eq!(want, got);
}

#[test]
fn an_optional_backlink_skips_none() {
    let f = fixture();
    let got =
        f.value("from u in users select { name: u.name, n: count(s for s in u.<sessions.owner) }");
    let want = set([
        rec(vec![("name", text("ann")), ("n", int(1))]),
        rec(vec![("name", text("bo")), ("n", int(0))]),
        rec(vec![("name", text("cy")), ("n", int(0))]),
    ]);
    assert_eq!(want, got);
}

#[test]
fn a_nested_result_keeps_rows_with_nothing_related() {
    let f = fixture();
    let got = f.value(
        "from u in users
         select { name: u.name,
                  phones: from d in phones where d.owner == key u select d }",
    );
    let phones = f.value("from d in phones select d");
    let want = set([
        rec(vec![("name", text("ann")), ("phones", phones)]),
        rec(vec![("name", text("bo")), ("phones", set([]))]),
        rec(vec![("name", text("cy")), ("phones", set([]))]),
    ]);
    assert_eq!(want, got);
}

#[test]
fn a_nested_set_flattens_by_ranging_over_it() {
    let f = fixture();
    let got = f.value(
        "from r in (from u in users select { name: u.name, ds: u.<phones.owner }),
              d in r.ds
         select { name: r.name, seen: d.seen }",
    );
    let want = set([
        rec(vec![("name", text("ann")), ("seen", t(10))]),
        rec(vec![("name", text("ann")), ("seen", t(20))]),
    ]);
    assert_eq!(want, got);
}

#[test]
fn group_into_partitions_bindings() {
    let f = fixture();
    let got = f.value(
        "from i in identities group i by i.user into g
         select { user: g.key, n: count(x for x in g) }",
    );
    let want = set([
        rec(vec![("user", key(1)), ("n", int(2))]),
        rec(vec![("user", key(2)), ("n", int(2))]),
    ]);
    assert_eq!(want, got);
}

#[test]
fn grouping_by_tag_groups_by_constructor() {
    let f = fixture();
    let got = f.value(
        "from i in identities group i by tag(i) into g
         select { kind: g.key, n: count(x for x in g) }",
    );
    let tag = |c| Value::Ctor(c, Vec::new());
    let want = set([
        rec(vec![("kind", tag(0)), ("n", int(1))]),
        rec(vec![("kind", tag(1)), ("n", int(2))]),
        rec(vec![("kind", tag(2)), ("n", int(1))]),
    ]);
    assert_eq!(want, got);
}

#[test]
fn an_aggregate_counts_equal_values_of_distinct_bindings() {
    let f = fixture();
    let got = f
        .eval_with(
            "sum(o.total for o in orders where o.user == key u)",
            &[("u", "User", user(&f, 1))],
        )
        .unwrap();
    assert_eq!(int(250), got);
}

#[test]
fn an_aggregate_over_a_group_is_over_a_nonempty_range() {
    let f = fixture();
    let got = f.value(
        "from o in orders group o by o.user into g
         select { user: g.key, top: max(x.total for x in g) }",
    );
    let want = set([
        rec(vec![("user", key(1)), ("top", int(100))]),
        rec(vec![("user", key(2)), ("top", int(30))]),
    ]);
    assert_eq!(want, got);
}

#[test]
fn an_aggregate_over_a_possibly_empty_range_is_optional() {
    let f = fixture();
    let got = f.value(
        "from u in users
         select { name: u.name,
                  top: max(o.total for o in orders where o.user == key u) }",
    );
    let want = set([
        rec(vec![("name", text("ann")), ("top", Value::some(int(100)))]),
        rec(vec![("name", text("bo")), ("top", Value::some(int(30)))]),
        rec(vec![("name", text("cy")), ("top", Value::none())]),
    ]);
    assert_eq!(want, got);
}

#[test]
fn an_aggregate_over_a_match_ranges_over_its_result() {
    let f = fixture();
    let got = f.value(
        "count(x for x in (from i in identities
                           select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub))",
    );
    assert_eq!(int(4), got);
}

const REACH: &str = "fix reach = (from f in follows where f.src == me select f.dst)
          union (from r in reach, f in follows where f.src == r select f.dst)
in from u in users where exists r in reach where r == key u select u";

#[test]
fn fix_is_the_least_fixpoint() {
    let f = fixture();
    let got = f.eval_with(REACH, &[("me", "Key users", key(1))]).unwrap();
    assert_eq!(set([user(&f, 2), user(&f, 3)]), got);
    let got = f.eval_with(REACH, &[("me", "Key users", key(2))]).unwrap();
    assert_eq!(set([user(&f, 2), user(&f, 3)]), got);
}

#[test]
fn mutual_recursion_is_one_fixpoint() {
    let f = fixture();
    let got = f
        .eval_with(
            "fix even = (from u in users where key u == me select key u)
                        union (from o in odd, f in follows where f.src == o select f.dst),
                 odd  = from v in even, f in follows where f.src == v select f.dst
             in from u in users where exists o in odd where o == key u select u",
            &[("me", "Key users", key(1))],
        )
        .unwrap();
    assert_eq!(set([user(&f, 2)]), got);
}

#[test]
fn a_lower_stratum_may_be_negated() {
    let f = fixture();
    let got = f
        .eval_with(
            "fix reach = (from f in follows where f.src == me select f.dst)
                       union (from r in reach, f in follows where f.src == r select f.dst)
             in from u in users where not exists r in reach where r == key u select u",
            &[("me", "Key users", key(1))],
        )
        .unwrap();
    assert_eq!(set([user(&f, 1)]), got);
}

#[test]
fn functions_and_views_are_called() {
    let f = fixture();
    let got =
        f.value("from u in active(), l in logins_of(key u) select { name: u.name, login: l }");
    let want = set([
        rec(vec![
            ("name", text("ann")),
            ("login", text("ann@example.com")),
        ]),
        rec(vec![("name", text("ann")), ("login", text("g-ann"))]),
        rec(vec![("name", text("bo")), ("login", text("root"))]),
        rec(vec![("name", text("bo")), ("login", text("a-bo"))]),
    ]);
    assert_eq!(want, got);
}

#[test]
fn a_function_takes_a_lambda() {
    let f = fixture();
    let got = f.value(
        "having(fn i -> match i | Google _ -> true | EmailPass _ -> false | Apple _ -> false)",
    );
    assert_eq!(keys([1, 2]), got);
}

#[test]
fn a_constructor_relation_reads_its_rows() {
    let f = fixture();
    let p = [("k", "Key identities", f.value(&uuid(3)))];
    assert_eq!(
        set([identity(&f, 3)]),
        f.eval_with("from i in identities where i.id == k select i", &p)
            .unwrap()
    );
    assert_eq!(
        texts(["g-ann", "root"]),
        f.value("from g in identities.Google select g.gid")
    );
}

#[test]
fn a_sum_parameter_is_compared_whole() {
    let f = fixture();
    let q = "query by_status (s: Status, since: Instant) -> Set Invoice =
               from i in invoices where i.status == s and i.issued >= since select i";
    let draft = f.value("Draft");
    let got = f.call_query(q, vec![draft, t(15)]).unwrap();
    let inv3 = row(
        &f,
        "from i in invoices where i.id == invoices.key(3) select i",
    );
    assert_eq!(set([inv3]), got);
    let sent = f
        .eval_with("Sent { at: t }", &[("t", "Instant", t(10))])
        .unwrap();
    assert_eq!(
        1,
        f.call_query(q, vec![sent, t(0)])
            .unwrap()
            .as_set()
            .unwrap()
            .len()
    );
}

#[test]
fn optional_filters_are_option_parameters() {
    let f = fixture();
    let q = "query search (name: Option Text, after: Option Instant) -> Set Post =
               from p in posts
               where match name  | None -> true | Some { value: n } -> p.title == n
               where match after | None -> true | Some { value: t } -> p.created_at > t
               select p";
    let got = f
        .call_query(q, vec![Value::some(text("again")), Value::none()])
        .unwrap();
    assert_eq!(set([post(&f, 2)]), got);
    let got = f
        .call_query(q, vec![Value::none(), Value::some(t(35))])
        .unwrap();
    assert_eq!(set([post(&f, 4), post(&f, 5)]), got);
    let got = f.call_query(q, vec![Value::none(), Value::none()]).unwrap();
    assert_eq!(5, got.as_set().unwrap().len());
}

#[test]
fn order_by_lists_the_final_result() {
    let f = fixture();
    let got = f.value("from u in users select u order by u.name");
    assert_eq!(
        Value::List(vec![user(&f, 1), user(&f, 2), user(&f, 3)]),
        got
    );
    let got = f.value("from u in users select u order by u.name desc");
    assert_eq!(
        Value::List(vec![user(&f, 3), user(&f, 2), user(&f, 1)]),
        got
    );
}

#[test]
fn top_takes_the_first_by_a_determinate_order() {
    let f = fixture();
    let got = f.value(
        "from u in users
         select { user: u,
                  latest: top 3 by (p.created_at desc, key p)
                            (from p in posts where p.author == key u select p) }",
    );
    let want = set([
        rec(vec![
            ("user", user(&f, 1)),
            ("latest", set([post(&f, 5), post(&f, 4), post(&f, 2)])),
        ]),
        rec(vec![("user", user(&f, 2)), ("latest", set([post(&f, 3)]))]),
        rec(vec![("user", user(&f, 3)), ("latest", set([]))]),
    ]);
    assert_eq!(want, got);
}

#[test]
fn after_starts_past_the_cursor() {
    let f = fixture();
    let q = "query page (cursor: Option (Key users)) -> Set User =
               from u in users select u order by u.id take 50 after cursor";
    assert_eq!(
        Value::List(vec![user(&f, 1), user(&f, 2), user(&f, 3)]),
        f.call_query(q, vec![Value::none()]).unwrap()
    );
    assert_eq!(
        Value::List(vec![user(&f, 2), user(&f, 3)]),
        f.call_query(q, vec![Value::some(key(1))]).unwrap()
    );
}

#[test]
fn a_compound_cursor_matches_keys_by_field_name() {
    let f = fixture();
    let q = "query feed_page (cursor: Option ({ at: Instant, id: Key posts })) -> Set Post =
               from p in posts select p order by (p.created_at desc, key p) take 20 after cursor";
    let all: Vec<Value> = [5, 4, 3, 2, 1].iter().map(|&i| post(&f, i)).collect();
    assert_eq!(
        Value::List(all.clone()),
        f.call_query(q, vec![Value::none()]).unwrap()
    );
    let cursor = Value::some(rec(vec![("at", t(40)), ("id", key(4))]));
    assert_eq!(
        Value::List(all[2..].to_vec()),
        f.call_query(q, vec![cursor]).unwrap()
    );
}

#[test]
fn a_guard_keeps_a_partial_operand_from_running() {
    let f = fixture();
    let got = f.value("from o in orders where o.qty > 0 and o.total / o.qty > 10 select o");
    let o1 = row(&f, "from o in orders where o.id == orders.key(1) select o");
    assert_eq!(set([o1]), got);
    assert_eq!(
        Err(adtb_reference::Abort::Fault(Fault::DivisionByZero)),
        f.eval("from o in orders where o.total / o.qty > 10 select o")
    );
}

#[test]
fn a_checked_operator_returns_none_where_it_would_fault() {
    let f = fixture();
    let got = f.value(
        "from o in orders
         select match checked_div(o.total, o.qty) | None -> 0 | Some { value: q } -> q",
    );
    assert_eq!(ints([50, 0, 10]), got);
}

#[test]
fn try_returns_the_fault_as_a_value() {
    let f = fixture();
    let got = f.value("from r in raws select { line: key r, n: try i32(r.n) }");
    let ok = Value::Ctor(0, vec![("value".into(), int(5))]);
    let err = Value::Ctor(1, vec![("error".into(), f.fault(Fault::OutOfRange))]);
    let want = set([
        rec(vec![("line", key(1)), ("n", ok)]),
        rec(vec![("line", key(2)), ("n", err)]),
    ]);
    assert_eq!(want, got);
}

#[test]
fn adhoc_anonymous_constructors_are_numbered_in_their_joined_sum() {
    let f = fixture();
    let zed = |n| Value::Ctor(1, vec![("n".into(), Value::Int(n))]);
    let alpha = Value::Ctor(0, Vec::new());
    let q = "match i | Google g -> Zed { n: 1 } | Apple a -> Alpha | EmailPass e -> Zed { n: 2 }";
    let at = |n| {
        f.eval_with(q, &[("i", "Identity", identity(&f, n))])
            .unwrap()
    };
    assert_eq!(zed(2), at(1));
    assert_eq!(zed(1), at(2));
    assert_eq!(alpha, at(4));
    assert_eq!(
        set([zed(1), alpha.clone()]),
        f.value("[Zed { n: 1 }, Alpha]")
    );
    assert_eq!(
        set([zed(1), alpha.clone()]),
        f.value("[Zed { n: 1 }] union [Alpha]")
    );
    assert_eq!(Value::Ctor(0, Vec::new()), f.value("Zed == Alpha"));
    let z = Value::Ctor(0, vec![("n".into(), Value::Int(3))]);
    assert_eq!(
        zed(3),
        f.eval_with("if true then z else Alpha", &[("z", "Zed { n: I64 }", z)])
            .unwrap()
    );
}
