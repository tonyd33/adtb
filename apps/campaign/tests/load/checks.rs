use super::fixture::{self, Fixture};
use super::{Calls, Ledger, Target, call};
use adtb_sim::SimRuntime;
use campaign::v1_6;

#[derive(Clone, Copy, Debug)]
pub enum Knowledge {
    Hidden,
    Revealing,
    Known,
}

impl Knowledge {
    fn permits(self, present: bool) -> bool {
        match self {
            Self::Hidden => !present,
            Self::Revealing => true,
            Self::Known => present,
        }
    }
}

pub fn privacy(calls: &Calls, fixture: &Fixture, knowledge: [Knowledge; 2]) {
    for (index, campaign) in fixture.campaigns.iter().enumerate() {
        let known: Vec<_> = call(calls, v1_6::known_entities, (campaign.party.clone(),))
            .map(Result::unwrap)
            .collect();
        assert!(!format!("{known:?}").contains("SECRET"));
        let mut ids = Vec::new();
        for row in &known {
            let v1_6::KnownEntitiesRow::Person {
                id, dead: false, ..
            } = row
            else {
                panic!("unexpected known entity {row:?}");
            };
            assert!(
                [&campaign.source, &campaign.targets[0], &campaign.targets[1]].contains(&id),
                "disclosed unknown key {id:?}"
            );
            ids.push(id.clone());
        }
        for target in &campaign.targets {
            assert!(ids.contains(target), "lost known target {target:?}");
        }
        let source_known = ids.contains(&campaign.source);
        assert!(
            knowledge[index].permits(source_known),
            "source list visibility differs from reveal phase {:?}",
            knowledge[index]
        );
        assert_eq!(ids.len(), if source_known { 3 } else { 2 });
        for (target, name) in [
            (&campaign.source, "Source"),
            (&campaign.targets[0], "A"),
            (&campaign.targets[1], "B"),
        ] {
            let rows: Vec<_> = call(
                calls,
                v1_6::known_entity,
                (campaign.party.clone(), target.clone()),
            )
            .map(Result::unwrap)
            .collect();
            assert!(!format!("{rows:?}").contains("SECRET"));
            assert!(rows.len() <= 1, "detail returns at most one entity");
            if target == &campaign.source {
                assert!(
                    knowledge[index].permits(!rows.is_empty()),
                    "source detail visibility differs from reveal phase {:?}",
                    knowledge[index]
                );
            } else {
                assert_eq!(rows.len(), 1, "a known target has a detail row");
            }
            for row in rows {
                assert_eq!(row.name, name, "detail belongs to its requested entity");
            }
        }
        for target in [&campaign.unknown, &fixture.campaigns[1 - index].source] {
            assert!(
                call(
                    calls,
                    v1_6::known_entity,
                    (campaign.party.clone(), target.clone())
                )
                .next()
                .is_none(),
                "unknown or other campaign entity was disclosed"
            );
        }
        let encounter: Vec<_> = call(calls, v1_6::party_encounter, (campaign.party.clone(),))
            .map(Result::unwrap)
            .collect();
        assert!(!format!("{encounter:?}").contains("SECRET"));
        if index == 0 {
            let [encounter] = encounter.as_slice() else {
                panic!("the live party sees exactly its fixture encounter: {encounter:?}");
            };
            assert_eq!(encounter.name, "Encounter");
            let mut order: Vec<_> = encounter
                .order
                .iter()
                .map(|row| (row.slot, row.label.as_str(), &row.standing))
                .collect();
            order.sort_unstable_by_key(|row| row.0);
            assert_eq!(
                order,
                vec![
                    (1, "Extra 0", &v1_6::Standing::Ready),
                    (2, "Extra 1", &v1_6::Standing::Ready)
                ]
            );
        } else {
            assert!(
                encounter.is_empty(),
                "the other party has no encounter: {encounter:?}"
            );
        }
    }
}

pub fn state(calls: &Calls, fixture: &Fixture, ledger: &Ledger, seed: u64) {
    let events = ledger.events();
    let turns = events
        .iter()
        .filter(|e| e.action == "next_turn" && e.attempts > 0)
        .count();
    assert_eq!(turns, 8, "seed={seed}");
    let tracker = call(calls, v1_6::tracker, (fixture.encounter.clone(),))
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(
        (tracker.round, tracker.turn),
        (1 + i64::try_from(turns / 2).unwrap(), 1),
        "seed={seed}"
    );
    let campaign = &fixture.campaigns[1];
    let mut numbers: Vec<_> = call(calls, v1_6::session_list, (campaign.id.clone(),))
        .map(|s| s.unwrap().number)
        .collect();
    numbers.sort_unstable();
    assert_eq!(numbers, vec![1, 2], "seed={seed}");
    let live: Vec<_> = call(calls, v1_6::live_session, (campaign.id.clone(),))
        .map(Result::unwrap)
        .collect();
    assert_eq!(live.len(), 1, "seed={seed}");
    let started = events
        .iter()
        .find(|e| e.action == "start_session" && e.position.is_some())
        .unwrap();
    assert_eq!(
        started.target,
        Target::Session(live[0].id.clone()),
        "seed={seed}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.action == "start_session" && e.attempts > 0)
            .count(),
        1,
        "seed={seed}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.action == "start_session" && e.attempts == 0)
            .count(),
        1,
        "seed={seed}"
    );
    assert!(
        events.iter().any(|e| e.attempts > 1),
        "seed={seed}: schedule exercised no engine retry"
    );
    for campaign in &fixture.campaigns {
        let page = call(calls, v1_6::entity_page, (campaign.source.clone(),))
            .next()
            .unwrap()
            .unwrap();
        let writes: Vec<_> = events
            .iter()
            .filter(|e| {
                e.action == "write_entity" && e.target == Target::Entity(campaign.source.clone())
            })
            .collect();
        assert_eq!(writes.len(), 2, "seed={seed}");
        assert_eq!(
            events
                .iter()
                .filter(|e| e.action == "reveal"
                    && e.target == Target::Entity(campaign.source.clone())
                    && e.position.is_some())
                .count(),
            2,
            "seed={seed}"
        );
        let last = writes
            .iter()
            .max_by_key(|event| event.position.unwrap())
            .unwrap();
        let actor = last.actor;
        assert_eq!(
            page.body,
            if actor == 0 { "A" } else { "B" },
            "seed={seed}: last committed write {last:?}"
        );
        assert_eq!(page.summary, format!("Writer {actor}"), "seed={seed}");
        assert_eq!(page.secret, format!("SECRET-{actor}"), "seed={seed}");
        assert_eq!(page.mentions.len(), 2, "seed={seed}");
        let shown = page.mentions.iter().find(|link| !link.hidden).unwrap();
        assert_eq!(shown.name, page.body, "seed={seed}");
        assert_eq!(shown.target, campaign.targets[actor], "seed={seed}");
        let hidden = page.mentions.iter().find(|link| link.hidden).unwrap();
        assert_eq!(hidden.target, campaign.targets[1 - actor], "seed={seed}");
        assert_eq!(page.known_by.len(), 1, "seed={seed}: duplicate reveal");
        let known = call(
            calls,
            v1_6::known_entity,
            (campaign.party.clone(), campaign.source.clone()),
        )
        .next()
        .unwrap()
        .unwrap();
        assert_eq!(known.body, page.body, "seed={seed}");
        assert_eq!(known.mentions.len(), 1, "seed={seed}");
        assert_eq!(known.mentions[0].target, shown.target, "seed={seed}");
    }
    privacy(calls, fixture, [Knowledge::Known; 2]);
}

pub fn reopen(rt: SimRuntime, fixture: &Fixture, ledger: &Ledger, seed: u64) {
    let db = fixture::open(rt);
    let calls = db.bundle(&v1_6::BUNDLE).unwrap();
    state(&calls, fixture, ledger, seed);
    db.close().unwrap();
}
