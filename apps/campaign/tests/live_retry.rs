#[path = "support/scratch.rs"]
mod support;

use adtb::{BundleDef, CallOptions, CreateOptions, Log, OpenOptions};
use campaign::{MIGRATIONS, v1_5};
use std::sync::Barrier;

#[test]
fn two_committed_next_turn_calls_must_advance_twice() {
    let path = support::scratch("live-retry-repro");
    let db = adtb::open(
        &path,
        &Log::new(MIGRATIONS),
        &OpenOptions {
            create: Some(CreateOptions::default()),
            workers: Some(2),
            registrations: vec![v1_5::BUNDLE.registration().unwrap()],
            ..OpenOptions::default()
        },
    )
    .unwrap();
    let bundle = db.bundle(&v1_5::BUNDLE).unwrap();
    let opts = CallOptions::default();
    let c = bundle
        .call(
            v1_5::new_campaign,
            ("Repro".into(), "".into(), "Party".into()),
            &opts,
        )
        .unwrap()
        .value;
    let p = bundle
        .call(v1_5::parties, (c.clone(),), &opts)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .id;
    let s = bundle
        .call(v1_5::plan_session, (c, p, "Session".into()), &opts)
        .unwrap()
        .value;
    bundle
        .call(v1_5::start_session, (s.clone(),), &opts)
        .unwrap();
    let e = bundle
        .call(
            v1_5::add_encounter,
            (s, "Encounter".into(), "".into()),
            &opts,
        )
        .unwrap()
        .value;
    let a = bundle
        .call(
            v1_5::add_extra,
            (e.clone(), "A".into(), "Extra".into()),
            &opts,
        )
        .unwrap()
        .value;
    let b = bundle
        .call(
            v1_5::add_extra,
            (e.clone(), "B".into(), "Extra".into()),
            &opts,
        )
        .unwrap()
        .value;
    bundle
        .call(
            v1_5::start_encounter,
            (
                e.clone(),
                vec![
                    v1_5::StartEncounterRollsArg {
                        combatant: a,
                        roll: 2,
                        slot: 1,
                    },
                    v1_5::StartEncounterRollsArg {
                        combatant: b,
                        roll: 1,
                        slot: 2,
                    },
                ],
            ),
            &opts,
        )
        .unwrap();
    for iteration in 0..32 {
        let barrier = Barrier::new(2);
        let (a, b) = std::thread::scope(|scope| {
            let a = scope.spawn(|| {
                barrier.wait();
                bundle.call(v1_5::next_turn, (e.clone(),), &CallOptions::default())
            });
            let b = scope.spawn(|| {
                barrier.wait();
                bundle.call(v1_5::next_turn, (e.clone(),), &CallOptions::default())
            });
            (a.join().unwrap().unwrap(), b.join().unwrap().unwrap())
        });
        let t = bundle
            .call(v1_5::tracker, (e.clone(),), &opts)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(
            (t.round, t.turn),
            (iteration + 2, 1),
            "iteration={iteration}, commits={:?}/{:?}, attempts={}/{}, order={:?}",
            a.position,
            b.position,
            a.attempts,
            b.attempts,
            t.order
        );
    }
    db.close().unwrap();
}
