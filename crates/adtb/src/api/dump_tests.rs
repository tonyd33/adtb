use adtb_lang::schema::prelude::ctor;

use super::*;
use crate::catalog::codec::testing::{position, reread, reversed, row, tag};
use crate::catalog::engine_schema;
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
