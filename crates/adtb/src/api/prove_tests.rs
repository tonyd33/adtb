//! A prove that cannot read its test database fails as storage and records no verdict.

use adtb_sim::{Action, Config, Match, Plan, Sim};
use adtb_testkit::process;

use crate::runtime::{IoError, OpKind};
use crate::test_world::load::{self as open, options};
use crate::{ArchiveTarget, CallOptions, Error, FatalReason, Log, OpenOptions, Proof};
use crate::{proof_in, prove_in};

const FILE: &str = "prove.adt";
const TARGET: &str = "bk";
const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
";

fn no_create() -> OpenOptions {
    OpenOptions {
        create: None,
        ..options(2)
    }
}

#[test]
fn a_prove_whose_test_database_does_not_open_fails_as_storage_and_records_no_verdict() {
    let mut sim = Sim::new(1, Config::default());
    process::run(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &options(2));
        let backup = format!("backup to \"{TARGET}\"");
        db.query_text(&backup, &[], &CallOptions::default())
            .unwrap();
        db.close().unwrap();
    });
    let archive = ArchiveTarget::path(TARGET).unwrap();
    let of_the_test_database = || {
        Match::kind(OpKind::Open)
            .containing("/restored.adt")
            .except(Match::any().containing("/restored.adt.partial-"))
            .except(Match::any().containing("/restored.adt-log"))
    };
    let (got, opens, proof) = process::run(&mut sim, |rt| {
        let opens = rt.logged(of_the_test_database());
        rt.plan(Plan::new().first(of_the_test_database(), Action::Fail(IoError::Failed)));
        let got = prove_in(rt.clone(), &archive, &Log::new([APP]), &no_create());
        (got, opens.entries(), proof_in(&rt, &archive))
    });
    let file = opens
        .first()
        .and_then(|o| o.path())
        .expect("the prove opens its test database");
    let detail = format!("`{file}`: other error");
    assert_eq!(
        Err(Error::Fatal {
            reason: FatalReason::Storage { detail }
        }),
        got
    );
    assert_eq!(Ok(Proof::Unproven { function: None }), proof);
}
