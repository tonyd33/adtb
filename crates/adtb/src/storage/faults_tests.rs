use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::runtime::{IoError, OpKind};
use adtb_sim::{Action, Config, Match, Outcome, Plan, Sim, SimRuntime};

#[derive(Clone, Copy, Debug)]
enum StorageFailure {
    Write,
    LogCreate,
}

fn fail_storage_operation(rt: SimRuntime, case: StorageFailure, cause: IoError) {
    rt.arm(match case {
        StorageFailure::Write => Match::kind(OpKind::Write),
        StorageFailure::LogCreate => Match::kind(OpKind::CreateNew).containing("-log/"),
    });
    rt.plan(Plan::new().at(0, Action::Fail(cause)));
    let options = crate::storage::Options {
        name: "fatal.adt".into(),
        segment_size: 64 * 1024,
        max_pages: 1024,
        ..crate::test_world::options(16)
    };
    crate::storage::Pager::create(rt, &options)
        .expect("a planted fatal I/O failure must stop creation");
}

#[test]
fn failed_storage_operations_preserve_their_typed_io_cause() {
    let mut ran = 0;
    for (case, cause) in [
        (StorageFailure::Write, IoError::Full),
        (StorageFailure::LogCreate, IoError::Failed),
    ] {
        let mut sim = Sim::new(32, Config::default());
        let caught = catch_unwind(AssertUnwindSafe(|| {
            sim.run(|rt| fail_storage_operation(rt, case, cause))
        }));
        let payload = caught.expect_err("the planted storage failure must stop the process");
        let failure = payload
            .downcast_ref::<crate::runtime::FatalIo>()
            .unwrap_or_else(|| {
                panic!(
                    "{case:?}: expected typed fatal I/O, found {:?}",
                    crate::write::panic_text(&*payload)
                )
            });
        assert_eq!(cause, failure.cause(), "{case:?}");
        let numbered = sim.numbered();
        let struck = numbered.struck().expect("the planted failure ran");
        let context = match struck.kind() {
            OpKind::Write => "write".to_owned(),
            _ => format!("create {}", struck.path().unwrap()),
        };
        assert_eq!(context, failure.context(), "{case:?}");
        assert_eq!(None, crate::write::panic_text(&*payload), "{case:?}");
        ran += 1;
    }
    assert_eq!(2, ran);
}

#[test]
fn genuine_storage_io_failures_are_accepted_as_process_crashes() {
    let mut ran = 0;
    for (case, cause) in [
        (StorageFailure::Write, IoError::Full),
        (StorageFailure::LogCreate, IoError::Failed),
    ] {
        let mut sim = Sim::new(33, Config::default());
        let outcome = adtb_harness::crash::run_process(&mut sim, |rt| {
            fail_storage_operation(rt, case, cause);
        });
        assert_eq!(Outcome::Crashed, outcome, "{case:?}");
        assert!(
            sim.numbered().struck().is_some(),
            "{case:?}: the planted failure ran"
        );
        ran += 1;
    }
    assert_eq!(2, ran);
}
