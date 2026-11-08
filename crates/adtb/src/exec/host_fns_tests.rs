//! Host functions in process over the simulator: bodies the program supplies at open, checked
//! against their declarations, called once per batch of rows from queries, writes and
//! constraints.

use crate::test_world::load as open;

use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::batch::{self, Batch, BatchBuilder, Type};
use crate::key::{Scalar, Value as Stored};
use crate::runtime::FatalIo;
use crate::runtime::{IoError, OpKind};
use crate::storage::Pager;
use crate::{
    CallOptions, Database, Determinism, Error, Fault, HostError, HostFns, Log, OpenOptions,
    RejectReason,
};
use adtb_sim::{Action, Config, Match, Plan, Sim, SimRuntime};

use crate::test_world::load::options;

const NOTES: &str = r#"module shop
add module shop
add type Note = { id: Key notes, body: Text }
add relation notes : Note key id generate sequence
define fn slugify (t: Text) -> Text = host "slugify" version "1.0" deterministic
define fn stamp (t: Text) -> Text = host "stamp" version "1.0"
"#;

/// Returns `f` over the text of each row of the batch's one column, as a batch of one text
/// column.
fn texts(b: &Batch, f: impl Fn(&str) -> String) -> Batch {
    let mut out = BatchBuilder::new(batch::Schema::new(vec![Type::Scalar(Scalar::Text)]));
    for i in 0..b.len() {
        let Stored::Text(t) = b.value(0, i) else {
            panic!("a text argument");
        };
        out.push_row(&[Stored::Text(f(&t))]);
    }
    out.finish()
}

fn slug(t: &str) -> String {
    t.to_lowercase().replace(' ', "-")
}

fn with(fns: HostFns) -> OpenOptions {
    OpenOptions {
        host_fns: fns,
        ..options(4)
    }
}

fn opened(rt: &SimRuntime, log: &[&str], fns: HostFns) -> Database<SimRuntime> {
    open::open_in(rt, open::FILE, &Log::new(log.iter().copied()), &with(fns))
        .unwrap_or_else(|e| panic!("the database opens: {e:?}"))
}

fn write(db: &Database<SimRuntime>, text: &str) -> Result<(), Error> {
    db.query_text(text, &[], &CallOptions::default()).map(drop)
}

/// Returns the host functions with `slugify` failing as `body` does.
fn failing(body: impl Fn(&Batch) -> Result<Batch, HostError> + Send + Sync + 'static) -> HostFns {
    let mut fns = HostFns::new();
    fns.register("slugify", "1.0", Determinism::Deterministic, body);
    fns
}

fn faulted(got: Result<Vec<crate::DynRows>, Error>) -> bool {
    matches!(
        got,
        Err(Error::Rejected {
            reason: RejectReason::Faulted {
                fault: Fault::FunctionFailed
            }
        })
    )
}

#[test]
fn a_host_callback_preserves_a_genuine_storage_failure() {
    let mut sim = Sim::new(8, Config::default());
    let mut returned = None;
    let caught = catch_unwind(AssertUnwindSafe(|| {
        sim.run(|rt| {
            rt.arm(Match::kind(OpKind::Write).prefixed("callback.adt"));
            rt.plan(Plan::new().at(0, Action::Fail(IoError::Full)));
            let pager_rt = rt.clone();
            let fns = failing(move |b| {
                let opts = crate::storage::Options {
                    name: "callback.adt".into(),
                    segment_size: 64 * 1024,
                    max_pages: 1024,
                    ..crate::test_world::options(16)
                };
                Pager::create(pager_rt.clone(), &opts)
                    .expect("the injected storage failure must stop creation");
                Ok(texts(b, slug))
            });
            let db = opened(&rt, &[NOTES], fns);
            write(&db, r#"insert notes { body: "a" }"#).expect("a note");
            returned = Some(db.query_text(
                "from n in notes select slugify(n.body)",
                &[],
                &CallOptions::default(),
            ));
        })
    }));
    let struck = sim.numbered().struck().map(|s| (s.at(), s.kind()));
    assert_eq!(Some((0, OpKind::Write)), struck, "the write failure fired");
    if let Some(got) = returned {
        assert!(faulted(got), "the swallowed fatal became FunctionFailed");
        panic!("the host catcher returned FunctionFailed instead of resuming fatal I/O");
    }
    let payload = caught.expect_err("fatal I/O escapes after the simulator's crash lifecycle");
    let failure = payload
        .downcast_ref::<FatalIo>()
        .expect("the original fatal I/O payload escapes the callback catcher");
    assert_eq!(IoError::Full, failure.cause());
    assert_eq!("write", failure.context());
}
