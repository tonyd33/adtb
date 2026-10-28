use super::checks::Knowledge;
use super::fixture::{self, Fixture};
use super::{Calls, Ledger, Target, call};
use adtb::runtime::Threads;
use adtb::{CallOptions, RejectReason};
use adtb_sim::SimRuntime;
use campaign::v1_6;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn parallel(
    rt: &SimRuntime,
    calls: &Calls,
    fixture: &Fixture,
    ledger: &Ledger,
    knowledge: [Knowledge; 2],
    operation: impl Fn(usize, &Calls, &Ledger) + Send + Sync + 'static,
) {
    let gate = Arc::new(AtomicUsize::new(0));
    let operation = Arc::new(operation);
    let mut threads = Vec::new();
    for actor in 0..3 {
        let gate = gate.clone();
        let operation = operation.clone();
        let calls = calls.clone();
        let ledger = ledger.clone();
        let fixture = fixture.clone();
        let runtime = rt.clone();
        threads.push(rt.spawn("campaign-actor", move || {
            gate.fetch_add(1, Ordering::SeqCst);
            while gate.load(Ordering::SeqCst) < 3 {
                runtime.yield_now();
            }
            if actor < 2 {
                operation(actor, &calls, &ledger);
            } else {
                for _ in 0..2 {
                    super::checks::privacy(&calls, &fixture, knowledge);
                    runtime.yield_now();
                }
            }
        }));
    }
    for thread in threads {
        rt.join(thread);
    }
}

fn discover_and_edit(
    rt: &SimRuntime,
    calls: &Calls,
    fixture: &Fixture,
    ledger: &Ledger,
    mut knowledge: [Knowledge; 2],
) {
    for (index, campaign) in fixture.campaigns.iter().enumerate() {
        knowledge[index] = Knowledge::Revealing;
        let reveal_campaign = campaign.clone();
        parallel(
            rt,
            calls,
            fixture,
            ledger,
            knowledge,
            move |actor, calls, ledger| {
                ledger.commit(
                    actor,
                    "reveal",
                    Target::Entity(reveal_campaign.source.clone()),
                    0,
                    calls.call(
                        v1_6::reveal,
                        (
                            reveal_campaign.party.clone(),
                            reveal_campaign.source.clone(),
                            None,
                        ),
                        &CallOptions::default(),
                    ),
                );
            },
        );
        knowledge[index] = Knowledge::Known;
        let campaign = campaign.clone();
        parallel(
            rt,
            calls,
            fixture,
            ledger,
            knowledge,
            move |actor, calls, ledger| {
                let (shown, hidden) = if actor == 0 { ("A", "B") } else { ("B", "A") };
                ledger.commit(
                    actor,
                    "write_entity",
                    Target::Entity(campaign.source.clone()),
                    0,
                    calls.call(
                        v1_6::write_entity,
                        (
                            campaign.id.clone(),
                            campaign.source.clone(),
                            format!("Writer {actor}"),
                            shown.into(),
                            format!("SECRET-{actor}"),
                            vec![shown.into()],
                            vec![hidden.into()],
                        ),
                        &CallOptions::default(),
                    ),
                );
            },
        );
    }
}

pub fn run(rt: SimRuntime, fixture: &Fixture, ledger: &Ledger, seed: u64) {
    let db = fixture::open(rt.clone());
    let calls = db.bundle(&v1_6::BUNDLE).unwrap();
    let knowledge = [Knowledge::Hidden; 2];
    let encounter = fixture.encounter.clone();
    parallel(
        &rt,
        &calls,
        fixture,
        ledger,
        knowledge,
        move |actor, calls, ledger| {
            for step in 0..4 {
                ledger.commit(
                    actor,
                    "next_turn",
                    Target::Encounter(encounter.clone()),
                    step,
                    calls.call(
                        v1_6::next_turn,
                        (encounter.clone(),),
                        &CallOptions::default(),
                    ),
                );
            }
        },
    );
    let campaign = fixture.campaigns[1].clone();
    parallel(
        &rt,
        &calls,
        fixture,
        ledger,
        knowledge,
        move |actor, calls, ledger| {
            ledger.commit(
                actor,
                "plan_session",
                Target::Campaign(campaign.id.clone()),
                0,
                calls.call(
                    v1_6::plan_session,
                    (
                        campaign.id.clone(),
                        campaign.party.clone(),
                        format!("Plan {actor}"),
                    ),
                    &CallOptions::default(),
                ),
            );
        },
    );
    let sessions: Vec<_> = call(
        &calls,
        v1_6::session_list,
        (fixture.campaigns[1].id.clone(),),
    )
    .map(|row| row.unwrap().id)
    .collect();
    assert_eq!(sessions.len(), 2, "seed={seed}: both plans commit");
    parallel(
        &rt,
        &calls,
        fixture,
        ledger,
        knowledge,
        move |actor, calls, ledger| match calls.call(
            v1_6::start_session,
            (sessions[actor].clone(),),
            &CallOptions::default(),
        ) {
            Ok(commit) => ledger.commit(
                actor,
                "start_session",
                Target::Session(sessions[actor].clone()),
                0,
                Ok(commit),
            ),
            Err(
                ref error @ v1_6::Error::Rejected {
                    reason: RejectReason::Violated { ref constraint, .. },
                },
            ) if constraint == "campaign::sessions.unique(campaign) in Live" => ledger.refused(
                actor,
                "start_session",
                Target::Session(sessions[actor].clone()),
                error,
            ),
            Err(error) => panic!("seed={seed} start_session actor={actor}: {error:?}"),
        },
    );
    discover_and_edit(&rt, &calls, fixture, ledger, knowledge);
    super::checks::state(&calls, fixture, ledger, seed);
    db.close().unwrap();
}
