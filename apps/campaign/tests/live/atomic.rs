use adtb::typed::{Bundle, Date};
use adtb::{CallOptions, Callable, Key, RejectReason};
use campaign::v1_6;
use std::fmt::Debug;
use std::sync::Barrier;

struct Fixture<'a> {
    bundle: &'a Bundle<v1_6::Bundle>,
    session: Key<v1_6::Sessions>,
    encounter: Key<v1_6::Encounters>,
    combatant: Key<v1_6::Combatants>,
}

impl<'a> Fixture<'a> {
    fn new(bundle: &'a Bundle<v1_6::Bundle>, session: Key<v1_6::Sessions>) -> Self {
        let opts = CallOptions::default();
        let encounter = bundle
            .call(
                v1_6::add_encounter,
                (session.clone(), "Atomic encounter".into(), "".into()),
                &opts,
            )
            .unwrap()
            .value;
        let combatant = bundle
            .call(
                v1_6::add_extra,
                (encounter.clone(), "A".into(), "Extra".into()),
                &opts,
            )
            .unwrap()
            .value;
        Self {
            bundle,
            session,
            encounter,
            combatant,
        }
    }
    fn call<C: Callable<Bundle = v1_6::Bundle>>(&self, c: C, args: C::Args) -> C::Output {
        self.bundle.call(c, args, &CallOptions::default()).unwrap()
    }
    fn finish(&self) {
        self.call(v1_6::end_encounter, (self.encounter.clone(), "".into()));
        assert!(
            self.call(v1_6::tracker, (self.encounter.clone(),))
                .next()
                .is_none()
        );
        let page = self
            .call(v1_6::session_page, (self.session.clone(),))
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(
            page.encounters
                .iter()
                .find(|encounter| encounter.id == self.encounter)
                .unwrap()
                .state,
            "Resolved"
        );
    }
    fn roll(&self) -> Vec<v1_6::StartEncounterRollsArg> {
        vec![v1_6::StartEncounterRollsArg {
            combatant: self.combatant.clone(),
            roll: 20,
            slot: 1,
        }]
    }
}

fn refused<T: Debug>(result: Result<T, v1_6::Error>, expected: &str) {
    match result {
        Err(v1_6::Error::Rejected {
            reason: RejectReason::UserError { message },
        }) => assert_eq!(message, expected),
        other => panic!("expected application rejection {expected:?}, got {other:?}"),
    }
}

pub fn stale_rosters_and_ended_lifecycles_are_rejected_inside_registered_calls(
    bundle: &Bundle<v1_6::Bundle>,
    session: Key<v1_6::Sessions>,
) {
    let f = Fixture::new(bundle, session);
    let stale = f.roll();
    let b = f
        .call(
            v1_6::add_extra,
            (f.encounter.clone(), "B".into(), "Extra".into()),
        )
        .value;
    let opts = CallOptions::default();
    refused(
        f.bundle
            .call(v1_6::start_encounter, (f.encounter.clone(), stale), &opts),
        "Enter one roll for every combatant",
    );
    assert!(
        f.call(v1_6::tracker, (f.encounter.clone(),))
            .next()
            .is_none()
    );
    let mut rolls = f.roll();
    rolls.push(v1_6::StartEncounterRollsArg {
        combatant: b,
        roll: 10,
        slot: 2,
    });
    f.call(v1_6::start_encounter, (f.encounter.clone(), rolls));
    refused(
        f.bundle.call(
            v1_6::add_extra,
            (f.encounter.clone(), "C".into(), "Extra".into()),
            &opts,
        ),
        "Encounter is not prepared",
    );
    let prepared = f
        .call(
            v1_6::add_encounter,
            (f.session.clone(), "Not started".into(), "".into()),
        )
        .value;
    f.call(
        v1_6::end_session,
        (f.session.clone(), Date(0), "Log".into(), "Recap".into()),
    );
    refused(
        f.bundle.call(
            v1_6::add_encounter,
            (f.session.clone(), "Late".into(), "".into()),
            &opts,
        ),
        "Session is already played",
    );
    refused(
        f.bundle.call(
            v1_6::write_session_prep,
            (f.session.clone(), "Late".into()),
            &opts,
        ),
        "Session is already played",
    );
    refused(
        f.bundle.call(
            v1_6::set_standing,
            (f.combatant.clone(), v1_6::Standing::Downed, "Late".into()),
            &opts,
        ),
        "Encounter is not underway",
    );
    refused(
        f.bundle.call(
            v1_6::end_session,
            (
                f.session.clone(),
                Date(1),
                "Overwrite".into(),
                "Overwrite".into(),
            ),
            &opts,
        ),
        "Session is not live",
    );
    refused(
        f.bundle.call(
            v1_6::add_extra,
            (prepared.clone(), "Late".into(), "Extra".into()),
            &opts,
        ),
        "Session is already played",
    );
    refused(
        f.bundle.call(
            v1_6::add_cast,
            (prepared, Key(999999), "Late".into()),
            &opts,
        ),
        "Session is already played",
    );
    refused(
        f.bundle
            .call(v1_6::next_turn, (f.encounter.clone(),), &opts),
        "Encounter is not underway",
    );
    let page = f
        .call(v1_6::session_page, (f.session.clone(),))
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(
        (page.state.as_str(), page.log.as_str(), page.recap.as_str()),
        ("Played", "Log", "Recap")
    );
    assert_eq!(
        page.encounters
            .iter()
            .find(|e| e.id == f.encounter)
            .unwrap()
            .state,
        "Resolved"
    );
}

#[derive(Clone, Copy, Debug)]
enum RosterStartOrder {
    Concurrent,
    StartFirst,
    AddFirst,
}

pub fn concurrent_roster_addition_and_start_cannot_both_commit(
    bundle: &Bundle<v1_6::Bundle>,
    session: Key<v1_6::Sessions>,
) {
    for order in [
        RosterStartOrder::Concurrent,
        RosterStartOrder::StartFirst,
        RosterStartOrder::AddFirst,
    ] {
        roster_addition_and_start(bundle, session.clone(), order);
        let following = Fixture::new(bundle, session.clone());
        following.call(
            v1_6::start_encounter,
            (following.encounter.clone(), following.roll()),
        );
        assert_eq!(
            following
                .call(v1_6::tracker, (following.encounter.clone(),))
                .next()
                .unwrap()
                .unwrap()
                .order
                .len(),
            1,
            "after {order:?}"
        );
        following.finish();
    }
}

fn roster_addition_and_start(
    bundle: &Bundle<v1_6::Bundle>,
    session: Key<v1_6::Sessions>,
    order: RosterStartOrder,
) {
    let f = Fixture::new(bundle, session);
    let start = || {
        f.bundle.call(
            v1_6::start_encounter,
            (f.encounter.clone(), f.roll()),
            &CallOptions::default(),
        )
    };
    let add = || {
        f.bundle.call(
            v1_6::add_extra,
            (f.encounter.clone(), "B".into(), "Extra".into()),
            &CallOptions::default(),
        )
    };
    let (start, add) = match order {
        RosterStartOrder::Concurrent => {
            let barrier = Barrier::new(2);
            std::thread::scope(|scope| {
                let start = scope.spawn(|| {
                    barrier.wait();
                    start()
                });
                let add = scope.spawn(|| {
                    barrier.wait();
                    add()
                });
                (start.join().unwrap(), add.join().unwrap())
            })
        }
        RosterStartOrder::StartFirst => (start(), add()),
        RosterStartOrder::AddFirst => {
            let add = add();
            (start(), add)
        }
    };
    match order {
        RosterStartOrder::StartFirst => assert!(start.is_ok(), "start={start:?}"),
        RosterStartOrder::AddFirst => assert!(add.is_ok(), "add={add:?}"),
        RosterStartOrder::Concurrent => {}
    }
    assert_ne!(start.is_ok(), add.is_ok(), "start={start:?}, add={add:?}");
    let roster = f
        .call(v1_6::roster, (f.encounter.clone(),))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let tracker = f
        .call(v1_6::tracker, (f.encounter.clone(),))
        .next()
        .transpose()
        .unwrap();
    if start.is_ok() {
        refused(add, "Encounter is not prepared");
        assert_eq!(roster.len(), 1);
        assert_eq!(tracker.unwrap().order.len(), 1);
    } else {
        refused(start, "Enter one roll for every combatant");
        assert_eq!(roster.len(), 2);
        assert!(tracker.is_none());
        let rolls = roster
            .into_iter()
            .zip(1_i64..)
            .map(|(combatant, slot)| v1_6::StartEncounterRollsArg {
                combatant: combatant.id,
                roll: 20,
                slot,
            })
            .collect();
        f.call(v1_6::start_encounter, (f.encounter.clone(), rolls));
    }
    f.finish();
}
