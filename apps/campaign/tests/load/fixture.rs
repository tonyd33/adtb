use super::{Calls, call};
use adtb::{BundleDef, CreateOptions, Database, Key, Log, OpenOptions};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use campaign::{MIGRATIONS, v1_6};
use std::collections::BTreeMap;

pub type Files = BTreeMap<String, Vec<u8>>;

#[derive(Clone)]
pub struct Campaign {
    pub id: Key<v1_6::Campaigns>,
    pub party: Key<v1_6::Entities>,
    pub source: Key<v1_6::Entities>,
    pub targets: [Key<v1_6::Entities>; 2],
    pub unknown: Key<v1_6::Entities>,
}

#[derive(Clone)]
pub struct Fixture {
    pub campaigns: [Campaign; 2],
    pub encounter: Key<v1_6::Encounters>,
}

pub fn open(rt: SimRuntime) -> Database<SimRuntime> {
    Database::open_in(
        rt,
        "campaign.adt",
        &Log::new(MIGRATIONS),
        &OpenOptions {
            create: Some(CreateOptions::default()),
            workers: Some(2),
            registrations: vec![v1_6::BUNDLE.registration().unwrap()],
            ..OpenOptions::default()
        },
    )
    .unwrap()
}

fn campaign(calls: &Calls, index: usize) -> Campaign {
    let id = call(
        calls,
        v1_6::new_campaign,
        (format!("Campaign {index}"), "Pitch".into(), "Party".into()),
    )
    .value;
    let party = call(calls, v1_6::parties, (id.clone(),))
        .next()
        .unwrap()
        .unwrap()
        .id;
    let npc = |name: &str| {
        call(
            calls,
            v1_6::add_npc,
            (id.clone(), name.into(), "Summary".into(), None),
        )
        .value
    };
    let source = npc("Source");
    let targets = [npc("A"), npc("B")];
    let unknown = npc("Unknown");
    for key in [&targets[0], &targets[1]] {
        call(calls, v1_6::reveal, (party.clone(), key.clone(), None));
    }
    Campaign {
        id,
        party,
        source,
        targets,
        unknown,
    }
}

pub fn population() -> (Files, Fixture) {
    let mut sim = Sim::new(0, Config::default());
    let mut fixture = None;
    assert_eq!(
        sim.run(|rt| {
            let db = open(rt);
            let calls = db.bundle(&v1_6::BUNDLE).unwrap();
            let campaigns = [campaign(&calls, 0), campaign(&calls, 1)];
            let c = &campaigns[0];
            let session = call(
                &calls,
                v1_6::plan_session,
                (c.id.clone(), c.party.clone(), "Live".into()),
            )
            .value;
            call(&calls, v1_6::start_session, (session.clone(),));
            let encounter = call(
                &calls,
                v1_6::add_encounter,
                (session, "Encounter".into(), "SECRET-SETUP".into()),
            )
            .value;
            let rolls = (0..2)
                .map(|slot| {
                    let combatant = call(
                        &calls,
                        v1_6::add_extra,
                        (encounter.clone(), format!("Extra {slot}"), "Extra".into()),
                    )
                    .value;
                    v1_6::StartEncounterRollsArg {
                        combatant,
                        roll: 20 - slot,
                        slot: slot + 1,
                    }
                })
                .collect();
            call(&calls, v1_6::start_encounter, (encounter.clone(), rolls));
            fixture = Some(Fixture {
                campaigns,
                encounter,
            });
            db.close().unwrap();
        }),
        Outcome::Finished
    );
    (sim.files(), fixture.unwrap())
}
