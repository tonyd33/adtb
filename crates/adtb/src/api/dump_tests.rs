use adtb_lang::ids::CatCtorId;
use adtb_lang::schema::prelude::ctor;

use super::*;
use crate::catalog::codec::testing::{position, reread, reversed, row, tag};
use crate::catalog::{engine_schema, layout};
use crate::exec;
use crate::key::Value;

fn none() -> Value {
    Value::Sum {
        ctor: ctor::NONE,
        fields: Vec::new(),
    }
}

/// Returns the value of `stored`, a stored row of constructor `c` of `r`.
fn val(schema: &Schema, r: RelId, c: u32, stored: &[Value]) -> Val {
    let fields = layout::row(schema, r, c)
        .iter()
        .zip(stored)
        .map(|((_, t), v)| exec::from_stored(&exec::conv(schema, t), v))
        .collect();
    Val::ctor(c, fields)
}

/// Returns the value of an active bundle row with id 4 named `name`.
fn bundle_row(schema: &Schema, name: &str) -> Val {
    let codec = Codec::<bundle::Row>::new(schema);
    let at = (codec.relation(), position(schema, &codec, k::BUNDLE));
    let stored = row(
        schema,
        at.0,
        at.1,
        &[
            ("id", Value::I64(4)),
            ("name", Value::Text(name.into())),
            ("family", Value::Text("web".into())),
            ("owner", Value::Text("ops".into())),
            ("pins", Value::I64(3)),
            ("hash", Value::Bytes(vec![0xab])),
            ("edition", Value::Text("2026-10".into())),
            ("catalog", none()),
            (
                "hides_new",
                Value::Sum {
                    ctor: ctor::FALSE,
                    fields: Vec::new(),
                },
            ),
            ("lease", none()),
            ("renewed", Value::Instant(77)),
            (
                "priority",
                tag(schema, at, "priority", k::PRIORITY_REGULAR, Vec::new()),
            ),
            ("memory_share", none()),
            ("workers", none()),
            (
                "state",
                tag(schema, at, "state", k::BUNDLE_ACTIVE, Vec::new()),
            ),
        ],
    );
    val(schema, at.0, at.1, &stored)
}

/// Returns a bundle entry's text after its header line, holding `bundle` and no other row.
fn entry(schema: &Schema, bundle: Val) -> String {
    let b = Bundled::new(schema);
    [
        (b.bundles.relation(), BTreeSet::from([bundle])),
        (b.registered.relation(), BTreeSet::new()),
        (b.feeds.relation(), BTreeSet::new()),
        (b.members.relation(), BTreeSet::new()),
    ]
    .iter()
    .map(|(r, rows)| document(schema, *r, rows, ""))
    .collect()
}

#[test]
fn a_dumped_bundle_reads_back_with_its_id_and_family() {
    let schema = engine_schema();
    let text = entry(&schema, bundle_row(&schema, "web-2"));
    let (id, family, mark, _) = read_bundle(&schema, &text, "web-2").expect("the entry reads");
    assert_eq!((4, "web".to_owned(), None), (id, family, mark));
}

#[test]
fn a_dumped_bundle_whose_name_holds_no_version_does_not_read() {
    let schema = engine_schema();
    let text = entry(&schema, bundle_row(&schema, "web"));
    assert_eq!(None, read_bundle(&schema, &text, "web").map(|(id, ..)| id));
}

#[test]
fn a_feed_row_sample_decodes_the_same_under_the_shipped_and_the_reversed_catalog() {
    let (shipped, reversed) = (engine_schema(), reversed());
    let (old, new) = (
        Codec::<FeedRow>::new(&shipped),
        Codec::<FeedRow>::new(&reversed),
    );
    let (was, now) = (
        position(&shipped, &old, k::FEED),
        position(&reversed, &new, k::FEED),
    );
    let stored = row(
        &shipped,
        old.relation(),
        was,
        &[
            ("id", Value::I64(6)),
            ("bundle", Value::I64(4)),
            ("name", Value::Text("f".into())),
            ("ordinal", Value::I32(1)),
            ("relation", Value::I64(233)),
            ("set", Value::Text("all".into())),
        ],
    );
    let read = reread(
        (&shipped, old.relation(), was),
        (&reversed, new.relation(), now),
        &stored,
    );
    let want = FeedRow { id: 6, bundle: 4 };
    assert_eq!(Ok(&want), old.decode(was, &stored).as_ref());
    assert_eq!(Ok(want), new.decode(now, &read));
}

#[test]
fn a_feed_member_row_sample_decodes_the_same_under_the_shipped_and_the_reversed_catalog() {
    let (shipped, reversed) = (engine_schema(), reversed());
    let (old, new) = (
        Codec::<FeedMemberRow>::new(&shipped),
        Codec::<FeedMemberRow>::new(&reversed),
    );
    let (was, now) = (
        position(&shipped, &old, k::FEED_MEMBER),
        position(&reversed, &new, k::FEED_MEMBER),
    );
    let stored = row(
        &shipped,
        old.relation(),
        was,
        &[
            ("feed", Value::I64(6)),
            ("relation", Value::I64(233)),
            ("ctor", Value::I64(173)),
        ],
    );
    let read = reread(
        (&shipped, old.relation(), was),
        (&reversed, new.relation(), now),
        &stored,
    );
    assert_eq!(Ok(FeedMemberRow { feed: 6 }), old.decode(was, &stored));
    assert_eq!(Ok(FeedMemberRow { feed: 6 }), new.decode(now, &read));
}

/// Returns the value of a grant row of kind `kind` with id `id`: `read` on `target` given by
/// `owner` to `web`, dormant when `dormant` is.
fn grant(schema: &Schema, kind: CatCtorId, id: i64, dormant: bool, target: Option<&str>) -> Val {
    let codec = Codec::<RoleGrant>::new(schema);
    let at = (codec.relation(), position(schema, &codec, k::ROLE_GRANT));
    let Value::Sum { ctor: kind, .. } = tag(schema, at, "kind", kind, Vec::new()) else {
        panic!("a grant kind is a sum");
    };
    let fields = layout::row(schema, at.0, at.1)
        .iter()
        .map(|(n, _)| match n.as_str() {
            "id" => Val::Int(id),
            "kind" => Val::ctor(kind, Vec::new()),
            "privilege" => Val::text("read"),
            "target" => target.map_or_else(Val::none, |t| Val::some(Val::text(t))),
            "grantee" => Val::text("web"),
            "grantor" => Val::text("owner"),
            "grant_option" => Val::bool(false),
            "dormant" => Val::bool(dormant),
            other => panic!("a grant has no field `{other}`"),
        })
        .collect();
    Val::ctor(at.1, fields)
}

#[test]
fn a_restore_reads_each_dormant_grant_and_refuses_one_with_no_target() {
    let schema = engine_schema();
    let codec = Codec::<RoleGrant>::new(&schema);
    let log = |id, dormant, target| grant(&schema, k::GRANT_LOG, id, dormant, target);
    let rows = BTreeSet::from([log(3, true, Some("app::t")), log(4, false, Some("app::u"))]);
    let want = Dormant {
        id: 3,
        grantee: "web".into(),
        privilege: "read".into(),
        target: "app::t".into(),
    };
    assert_eq!(Some(vec![want]), dormant_grants(&schema, &codec, &rows));
    let untargeted = BTreeSet::from([log(3, true, None)]);
    assert_eq!(None, dormant_grants(&schema, &codec, &untargeted));
}

#[test]
fn a_restore_gives_each_replayed_log_grant_the_id_of_the_dumped_grant_it_pairs_with() {
    let schema = engine_schema();
    let codec = Codec::<RoleGrant>::new(&schema);
    let log = |id, dormant| grant(&schema, k::GRANT_LOG, id, dormant, Some("app::t"));
    let deployed = grant(&schema, k::GRANT_DEPLOYMENT, 2, false, None);
    let replayed = vec![log(1, false), deployed.clone()];
    assert_eq!(
        Some(BTreeSet::from([log(9, false), deployed])),
        paired(
            &schema,
            &codec,
            replayed.clone(),
            BTreeSet::from([log(9, false)])
        )
    );
    assert_eq!(
        Some(BTreeSet::from([log(9, true)])),
        paired(
            &schema,
            &codec,
            vec![log(1, true)],
            BTreeSet::from([log(9, false)])
        )
    );
    assert_eq!(
        None,
        paired(
            &schema,
            &codec,
            replayed.clone(),
            BTreeSet::from([log(9, true)])
        ),
        "a dormant dumped grant pairs only with a dormant replayed one"
    );
    assert_eq!(
        None,
        paired(
            &schema,
            &codec,
            replayed,
            BTreeSet::from([log(9, false), log(8, true)])
        ),
        "every dumped grant pairs"
    );
}
