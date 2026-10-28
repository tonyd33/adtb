//! The schema opened in the engine, and the bundles' queries and transactions called through the
//! code generated from them.

use std::fmt::Debug;
use std::path::PathBuf;

use adtb::Value;
use adtb::typed::{Date, HostValue, Relation};
use adtb::{
    BundleDef, CallOptions, Callable, CreateOptions, Database, Key, Log, OpenOptions, RejectReason,
    RetryReason, Rows, TxnCallOptions, TxnOptions,
};
use adtb_lang::check::{Checker, Context};
use adtb_lang::parse::{Options, parse_type};
use adtb_lang::resolve::Visible;
use adtb_lang::textual::{Form, parse, print};
use adtb_lang::types::Ty;
use campaign::v1_3::{Campaigns, Entities, Quests, Sessions};
use campaign::{MIGRATIONS, v1_0, v1_3, v1_4};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("campaign")
        .join(name);
    match std::fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => panic!("clear {dir:?}: {e}"),
        _ => {}
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("campaign.adt")
}

/// Open or create the database at `path` at the first `n` migrations, registering `bundles`.
fn open_db(path: PathBuf, n: usize, bundles: Vec<adtb::Registration>) -> Database {
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(2),
        registrations: bundles,
        ..OpenOptions::default()
    };
    let log = Log::new(MIGRATIONS[..n].iter().copied());
    adtb::open(path, &log, &options).unwrap_or_else(|e| panic!("open: {e:?}"))
}

/// Open or create the database at `path` at the first `n` migrations, registering no bundle,
/// and close it again.
fn migrate(path: PathBuf, n: usize) {
    open_db(path, n, Vec::new()).close().expect("close");
}

#[test]
#[ignore = "measure migration apply and replay with --nocapture"]
#[expect(
    clippy::disallowed_methods,
    reason = "measuring host elapsed time would need an external benchmark harness"
)]
fn migration_open_timings() {
    let path = scratch("open-timings");
    let start = std::time::Instant::now();
    let db = open_db(path.clone(), MIGRATIONS.len(), Vec::new());
    eprintln!("campaign current fresh apply: {:?}", start.elapsed());
    db.close().expect("close fresh database");
    let start = std::time::Instant::now();
    let db = open_db(path, MIGRATIONS.len(), Vec::new());
    eprintln!("campaign current reopen: {:?}", start.elapsed());
    db.close().expect("close reopened database");
}

struct App<B> {
    db: Database,
    bundle: adtb::typed::Bundle<B>,
}

type Current = App<v1_3::Bundle>;

impl<B: BundleDef> App<B> {
    /// Open or create the database at `path` at the first `n` migrations with bundle `def`.
    fn at(path: PathBuf, n: usize, def: &B) -> App<B> {
        let reg = def
            .registration()
            .expect("the generated bundle file registers");
        let db = open_db(path, n, vec![reg]);
        let bundle = db
            .bundle(def)
            .unwrap_or_else(|e| panic!("{}: {e:?}", B::NAME));
        App { db, bundle }
    }

    fn try_call<C: Callable<Bundle = B>>(
        &self,
        c: C,
        args: C::Args,
    ) -> Result<C::Output, B::Error> {
        self.bundle.call(c, args, &CallOptions::default())
    }

    fn call<C: Callable<Bundle = B>>(&self, c: C, args: C::Args) -> C::Output {
        self.try_call(c, args)
            .unwrap_or_else(|e| panic!("`{}` fails: {e:?}", std::any::type_name::<C>()))
    }

    fn adhoc(&self, text: &str) -> Result<Vec<String>, adtb::Error> {
        adhoc(&self.db, text, &[])
    }
}

impl Current {
    /// Open a new database under `name` at release 1.3.
    fn open(name: &str) -> Current {
        App::at(scratch(name), 4, &v1_3::BUNDLE)
    }
}

/// Returns the rows of ad hoc `text`, printed, each parameter given as its name, type and value
/// in the textual syntax.
fn adhoc(
    db: &Database,
    text: &str,
    given: &[(&str, &str, &str)],
) -> Result<Vec<String>, adtb::Error> {
    let schema = db.schema();
    let c = Checker::new(schema, Visible::all(schema), Context::Adhoc);
    let params: Vec<(&str, Ty, Value)> = given
        .iter()
        .map(|&(name, ty, v)| {
            let ty = parse_type(ty, Options::default()).ok().expect("a type");
            let ty = c.resolve_type(&ty).expect("the type resolves");
            let v = parse(v, &Form::of(schema, &ty)).expect("the value reads");
            (name, ty, v)
        })
        .collect();
    let results = db.query_text(text, &params, &CallOptions::default())?;
    Ok(results
        .into_iter()
        .flat_map(|rows| {
            let form = rows.form().clone();
            rows.map(move |r| print(&r.expect("a row"), &form).expect("a row prints"))
        })
        .collect())
}

fn text(t: &str) -> String {
    format!("{t:?}")
}

fn some(v: &str) -> String {
    format!("Some {{ value: {v} }}")
}

/// Returns every row of a call's result.
fn all<T: HostValue, E: From<adtb::Error> + Debug>(rows: Rows<T, E>) -> Vec<T> {
    rows.collect::<Result<_, _>>()
        .expect("every row is a value of its host type")
}

/// Returns the one row of a call's result.
fn one<T: HostValue + Debug, E: From<adtb::Error> + Debug>(rows: Rows<T, E>) -> T {
    let mut rows = all(rows);
    assert_eq!(1, rows.len(), "one row: {rows:?}");
    rows.remove(0)
}

/// Returns `k` as a key of the relation of the same name at another version.
fn carry<A, B>(k: Key<A>) -> Key<B>
where
    A: Relation<KeyValue = i64>,
    B: Relation<KeyValue = i64>,
{
    Key(k.0)
}

/// Returns `k` in the textual syntax, as ad hoc text takes it.
fn key<R: Relation<KeyValue = i64>>(k: &Key<R>) -> String {
    let rel = R::NAME.rsplit("::").next().expect("a relation name");
    format!("{rel}.key({})", k.0)
}

/// Returns whether `got` failed on a violation of constraint `name`.
fn violates<T: Debug>(got: &Result<T, v1_3::Error>, name: &str) -> bool {
    matches!(got, Err(v1_3::Error::Rejected { reason }) if violated(reason, name))
}

/// Returns whether ad hoc text failed `got` on a violation of constraint `name`.
fn adhoc_violates<T: Debug>(got: &Result<T, adtb::Error>, name: &str) -> bool {
    matches!(got, Err(adtb::Error::Rejected { reason }) if violated(reason, name))
}

fn violated(reason: &RejectReason, name: &str) -> bool {
    matches!(reason, RejectReason::Violated { constraint, .. }
        if constraint.rsplit("::").next() == Some(name))
}

/// A campaign with one party, a location and an NPC there.
#[derive(Clone, Debug)]
struct World {
    campaign: Key<Campaigns>,
    party: Key<Entities>,
    keep: Key<Entities>,
    warden: Key<Entities>,
}

fn world(app: &Current) -> World {
    let campaign = app
        .call(
            v1_3::new_campaign,
            (
                "Ashes of Vael".into(),
                "A fallen empire".into(),
                "The Lanterns".into(),
            ),
        )
        .value;
    let party = one(app.call(v1_3::lookup, (campaign.clone(), "Lanterns".into()))).id;
    let keep = app
        .call(
            v1_3::add_settlement,
            (
                campaign.clone(),
                "Greyhold".into(),
                "A keep on the pass".into(),
                None,
            ),
        )
        .value;
    let warden = app
        .call(
            v1_3::add_npc,
            (
                campaign.clone(),
                "Warden Ilsa".into(),
                "Holds the pass".into(),
                Some(keep.clone()),
            ),
        )
        .value;
    World {
        campaign,
        party,
        keep,
        warden,
    }
}

/// Build the world of [`world`] through the bundle of release 1.0.
fn world_1_0(app: &App<v1_0::Bundle>) -> World {
    let campaign = app
        .call(
            v1_0::new_campaign,
            (
                "Ashes of Vael".into(),
                "A fallen empire".into(),
                "The Lanterns".into(),
            ),
        )
        .value;
    let party = one(app.call(v1_0::lookup, (campaign.clone(), "Lanterns".into()))).id;
    let keep = app
        .call(
            v1_0::add_location,
            (
                campaign.clone(),
                "Greyhold".into(),
                "A keep on the pass".into(),
                None,
                v1_0::Scale::Town,
            ),
        )
        .value;
    let warden = app
        .call(
            v1_0::add_npc,
            (
                campaign.clone(),
                "Warden Ilsa".into(),
                "Holds the pass".into(),
                Some(keep.clone()),
            ),
        )
        .value;
    World {
        campaign: carry(campaign),
        party: carry(party),
        keep: carry(keep),
        warden: carry(warden),
    }
}

/// Assert that bundle `B` pins the version it names by the hash of that version.
fn pins_its_hash<B: BundleDef>() {
    let pins = usize::try_from(B::PINS).expect("a small version");
    let source =
        adtb_lang::signature::replay(MIGRATIONS[..pins].iter().copied()).expect("the log replays");
    let schema = source.schema().expect("the source builds");
    let hash: String = adtb::catalog::version_hash(&schema, &source)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert!(
        B::TEXT.lines().next().is_some_and(|h| h.contains(&hash)),
        "the header of {} states the hash {hash}",
        B::NAME
    );
}

#[test]
fn the_bundle_pins_version_1_by_its_hash() {
    assert_eq!(1, v1_0::Bundle::PINS);
    pins_its_hash::<v1_0::Bundle>();
}

#[test]
fn the_bundle_pins_version_4_by_its_hash() {
    assert_eq!(4, v1_3::Bundle::PINS);
    pins_its_hash::<v1_3::Bundle>();
}

#[test]
fn release_1_0_opens() {
    let app = App::at(scratch("opens"), 1, &v1_0::BUNDLE);
    assert!(all(app.call(v1_0::campaigns, ())).is_empty());
}

#[test]
fn release_1_1_opens_over_release_1_0() {
    let path = scratch("upgrade");
    let old = App::at(path.clone(), 1, &v1_0::BUNDLE);
    let w = world_1_0(&old);
    old.db.close().expect("close");
    let db = open_db(path, 2, Vec::new());
    let found = adhoc(
        &db,
        "from e in entities where contains(fold(e.name), \"ilsa\") select key e",
        &[],
    );
    assert_eq!(vec![key(&w.warden)], found.expect("the warden"));
    let map = adhoc(
        &db,
        &format!(
            r#"transact
             let m = insert maps {{ campaign: {c}, title: "The pass", image: "pass.png", width: 800, height: 600 }}
             insert pins.OfEntity {{ map: key m, x: 120, y: 340, entity: {keep} }}
             insert discoveries.KnowsMap {{ party: party, learned_in: None, map: key m }}
             return key m"#,
            c = key(&w.campaign),
            keep = key(&w.keep),
        ),
        &[("party", "Key entities.Party", &key(&w.party))],
    );
    assert!(map.is_ok(), "{map:?}");
    let off = adhoc(
        &db,
        &format!(
            "insert pins.Marker {{ map: {}, x: 900, y: 10, label: \"Off the edge\" }}",
            map.expect("a map")[0]
        ),
        &[],
    );
    assert!(adhoc_violates(&off, "pin_inside_map"), "{off:?}");
}

#[test]
fn prep_builds_a_world_the_gm_can_look_up() {
    let app = Current::open("prep");
    let w = world(&app);
    let found = one(app.call(v1_3::lookup, (w.campaign.clone(), "ilsa".into())));
    assert_eq!(
        ("Warden Ilsa", "NPC"),
        (found.name.as_str(), found.kind.as_str())
    );
    app.call(
        v1_3::write_entity,
        (
            w.warden.clone(),
            "Holds the pass".into(),
            "She keeps [[Greyhold]].".into(),
            "Sold the pass to [[The Lanterns]].".into(),
            vec![w.keep.clone()],
            vec![w.party.clone()],
        ),
    );
    let page = one(app.call(v1_3::entity_page, (w.warden.clone(),)));
    let mentions: Vec<(&str, bool)> = page
        .mentions
        .iter()
        .map(|m| (m.name.as_str(), m.hidden))
        .collect();
    assert_eq!(
        vec![("Greyhold", false), ("The Lanterns", true)],
        mentions,
        "{page:?}"
    );
    let keep = one(app.call(v1_3::entity_page, (w.keep.clone(),)));
    assert_eq!(
        vec![v1_3::EntityPageRowMentionedIn::InEntity {
            id: w.warden,
            name: "Warden Ilsa".into()
        }],
        keep.mentioned_in,
        "the keep is mentioned by the warden"
    );
}

#[test]
fn text_edit_preserves_existing_shared_and_hidden_links() {
    let app = App::at(scratch("text-links"), 4, &v1_4::BUNDLE);
    let campaign = app
        .call(
            v1_4::new_campaign,
            ("Campaign".into(), "Pitch".into(), "Party".into()),
        )
        .value;
    let party = one(app.call(v1_4::parties, (campaign.clone(),))).id;
    let keep = app
        .call(
            v1_4::add_region,
            (campaign.clone(), "Keep".into(), "Region".into(), None),
        )
        .value;
    let warden = app
        .call(
            v1_4::add_npc,
            (campaign, "Warden".into(), "Summary".into(), None),
        )
        .value;
    app.call(
        v1_4::write_entity,
        (
            warden.clone(),
            "Old".into(),
            "Shared".into(),
            "Secret".into(),
            vec![keep],
            vec![party],
        ),
    );
    let before = one(app.call(v1_4::entity_page, (warden.clone(),)));
    app.call(
        v1_4::write_entity_text,
        (
            warden.clone(),
            "New summary".into(),
            "New shared".into(),
            "New secret".into(),
        ),
    );
    let after = one(app.call(v1_4::entity_page, (warden,)));
    assert_eq!(after.mentions, before.mentions);
    assert_eq!(
        (
            after.summary.as_str(),
            after.body.as_str(),
            after.secret.as_str()
        ),
        ("New summary", "New shared", "New secret")
    );
}

#[test]
fn players_see_only_what_their_party_discovered() {
    let app = Current::open("discovery");
    let w = world(&app);
    app.call(
        v1_3::write_entity,
        (
            w.warden.clone(),
            "Holds the pass".into(),
            "She keeps [[Greyhold]].".into(),
            "Sold the pass.".into(),
            vec![w.keep.clone()],
            Vec::new(),
        ),
    );
    assert!(all(app.call(v1_3::known_entities, (w.party.clone(),))).is_empty());
    let reveal = (w.party.clone(), w.warden.clone(), None);
    app.call(v1_3::reveal, reveal.clone());
    app.call(v1_3::reveal, reveal);
    let known = one(app.call(v1_3::known_entities, (w.party.clone(),)));
    assert_eq!(
        v1_3::KnownEntitiesRow::Person {
            dead: false,
            id: w.warden.clone(),
            name: "Warden Ilsa".into(),
            summary: "Holds the pass".into()
        },
        known
    );
    let page = one(app.call(v1_3::known_entity, (w.party.clone(), w.warden.clone())));
    assert_eq!(
        "She keeps [[Greyhold]].", page.body,
        "the page shows the body and no secret"
    );
    assert!(
        page.mentions.is_empty(),
        "an undiscovered link stays hidden: {page:?}"
    );
    app.call(v1_3::reveal, (w.party.clone(), w.keep.clone(), None));
    let page = one(app.call(v1_3::known_entity, (w.party.clone(), w.warden)));
    let mentions: Vec<&str> = page.mentions.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(vec!["Greyhold"], mentions, "a discovered link shows");
}

#[test]
fn a_discovery_never_crosses_campaigns() {
    let app = Current::open("crossing");
    let w = world(&app);
    let other = app
        .call(
            v1_3::new_campaign,
            ("Other".into(), String::new(), "The Others".into()),
        )
        .value;
    let stranger = app
        .call(
            v1_3::add_npc,
            (other, "Stranger".into(), String::new(), None),
        )
        .value;
    let refused = app.try_call(v1_3::reveal, (w.party, stranger, None));
    assert!(
        violates(&refused, "entity_discovery_in_campaign"),
        "{refused:?}"
    );
}

/// Plans and starts session 1 of `w`, returning its key.
fn live_session(app: &Current, w: &World) -> Key<Sessions> {
    let s = app
        .call(
            v1_3::plan_session,
            (w.campaign.clone(), w.party.clone(), "The pass".into()),
        )
        .value;
    app.call(v1_3::start_session, (s.clone(),));
    s
}

/// Returns the round and turn of encounter `e`.
fn round_and_turn(app: &Current, e: &Key<v1_3::Encounters>) -> (i64, i64) {
    let t = one(app.call(v1_3::tracker, (e.clone(),)));
    (t.round, t.turn)
}

#[test]
fn an_encounter_runs_round_by_round() {
    let app = Current::open("encounter");
    let w = world(&app);
    let s = live_session(&app, &w);
    let e = app
        .call(
            v1_3::add_encounter,
            (s, "Ambush".into(), "Bandits on the road".into()),
        )
        .value;
    let ilsa = app
        .call(v1_3::add_cast, (e.clone(), w.warden.clone(), "Ilsa".into()))
        .value;
    let bandit = app
        .call(
            v1_3::add_extra,
            (e.clone(), "Bandit".into(), "bandit".into()),
        )
        .value;
    let roll = |combatant: &Key<v1_3::Combatants>, roll, slot| v1_3::StartEncounterRollsArg {
        combatant: combatant.clone(),
        roll,
        slot,
    };
    app.call(
        v1_3::start_encounter,
        (e.clone(), vec![roll(&ilsa, 17, 1), roll(&bandit, 9, 2)]),
    );
    assert_eq!((1, 1), round_and_turn(&app, &e));
    app.call(v1_3::next_turn, (e.clone(),));
    assert_eq!((1, 2), round_and_turn(&app, &e));
    app.call(v1_3::next_turn, (e.clone(),));
    assert_eq!((2, 1), round_and_turn(&app, &e));
    let wolf = app
        .call(v1_3::add_extra, (e.clone(), "Wolf".into(), "wolf".into()))
        .value;
    app.call(v1_3::join_fight, (e.clone(), wolf, 4, 3));
    app.call(v1_3::leave_fight, (ilsa,));
    assert_eq!((2, 2), round_and_turn(&app, &e), "the acting slot left");
    let seen = one(app.call(v1_3::party_encounter, (w.party,)));
    let order: Vec<(i64, &str)> = seen
        .order
        .iter()
        .map(|t| (t.slot, t.label.as_str()))
        .collect();
    assert_eq!(
        vec![(2, "Bandit"), (3, "Wolf")],
        order,
        "players see labels, not kinds"
    );
    app.call(v1_3::end_encounter, (e.clone(), "Bandits fled".into()));
    assert!(all(app.call(v1_3::tracker, (e,))).is_empty());
}

#[test]
fn a_second_live_session_is_refused() {
    let app = Current::open("two-live");
    let w = world(&app);
    live_session(&app, &w);
    let s2 = app
        .call(
            v1_3::plan_session,
            (w.campaign.clone(), w.party, "Again".into()),
        )
        .value;
    let refused = app.try_call(v1_3::start_session, (s2,));
    assert!(
        matches!(refused, Err(v1_3::Error::Rejected { .. })),
        "{refused:?}"
    );
}

#[test]
fn the_dead_do_not_fight_in_later_sessions() {
    let app = Current::open("dead");
    let w = world(&app);
    let s1 = live_session(&app, &w);
    app.call(v1_3::kill, (w.warden.clone(), s1.clone()));
    app.call(
        v1_3::end_session,
        (s1, Date(20750), "Ilsa fell.".into(), "Ilsa fell.".into()),
    );
    let s2 = live_session(&app, &w);
    let e = app
        .call(v1_3::add_encounter, (s2, "Haunting".into(), String::new()))
        .value;
    let refused = app.try_call(v1_3::add_cast, (e, w.warden.clone(), "Ilsa".into()));
    assert!(violates(&refused, "the_dead_stay_dead"), "{refused:?}");
    let recap = one(app.call(v1_3::recaps, (w.party,)));
    assert_eq!(
        (1, Date(20750), "Ilsa fell."),
        (recap.number, recap.played_on, recap.recap.as_str())
    );
}

#[test]
fn a_closed_quest_stays_closed() {
    let app = Current::open("quest");
    let w = world(&app);
    let s = live_session(&app, &w);
    let q = app
        .call(
            v1_3::add_quest,
            (
                w.campaign.clone(),
                "Clear the pass".into(),
                String::new(),
                Some(w.warden.clone()),
                None,
            ),
        )
        .value;
    app.call(v1_3::take_quest, (q.clone(), s.clone()));
    app.call(
        v1_3::close_quest,
        (q.clone(), s, false, "The pass fell".into()),
    );
    let log = one(app.call(v1_3::quest_log, (w.campaign,)));
    assert!(
        matches!(&log, v1_3::QuestLogRow::Done { won: false, outcome, closed: 1, .. } if outcome == "The pass fell"),
        "{log:?}"
    );
    let reopened = app.adhoc(&format!(
        "update x in quests.Failed where key x == {} set Active {{ taken_in: x.closed_in }}",
        key(&q)
    ));
    assert!(
        adhoc_violates(&reopened, "closed_quests_stay_closed"),
        "{reopened:?}"
    );
}

#[test]
fn two_gms_planning_the_same_session_number_do_not_both_commit() {
    let app = Current::open("collide");
    let w = world(&app);
    let args = (w.campaign.clone(), w.party.clone(), "Concurrent".into());
    let mut a = app.bundle.begin(&TxnOptions::default()).expect("begin a");
    a.call(v1_3::plan_session, args.clone(), &TxnCallOptions::default())
        .expect("a plans");
    let (a_done, b_done) = std::thread::scope(|s| {
        let (begun, wait) = std::sync::mpsc::channel();
        let bundle = &app.bundle;
        let b = s.spawn(move || {
            let mut b = bundle.begin(&TxnOptions::default())?;
            begun.send(()).expect("the test waits");
            b.call(v1_3::plan_session, args, &TxnCallOptions::default())?;
            b.commit(None)
        });
        wait.recv().expect("b begins");
        let a_done = a.commit(None);
        (a_done, b.join().expect("b runs"))
    });
    assert!(a_done.is_ok(), "{a_done:?}");
    assert!(
        matches!(
            b_done,
            Err(v1_3::Error::Retry {
                reason: RetryReason::Serialization { .. }
            })
        ),
        "the second plan of session 1 fails to serialize: {b_done:?}"
    );
    let numbers = app
        .adhoc(&format!(
            "from s in sessions where s.campaign == {} select s.number",
            key(&w.campaign)
        ))
        .expect("the sessions");
    assert_eq!(vec!["1"], numbers);
}

/// Returns the quest log of campaign `c`.
fn quest_log(app: &Current, c: &Key<Campaigns>) -> Vec<v1_3::QuestLogRow> {
    all(app.call(v1_3::quest_log, (c.clone(),)))
}

#[test]
fn release_1_3_shelves_a_quest_over_release_1_1() {
    let path = scratch("shelve");
    let old = App::at(path.clone(), 1, &v1_0::BUNDLE);
    let w = world_1_0(&old);
    old.db.close().expect("close");
    migrate(path.clone(), 2);
    let app = App::at(path, 4, &v1_3::BUNDLE);
    let s = live_session(&app, &w);
    let q: Key<Quests> = app
        .call(
            v1_3::add_quest,
            (
                w.campaign.clone(),
                "Find the heir".into(),
                String::new(),
                None,
                None,
            ),
        )
        .value;
    app.call(v1_3::take_quest, (q.clone(), s.clone()));
    app.call(v1_3::shelve_quest, (q.clone(), "winter".into()));
    assert_eq!(
        vec![v1_3::QuestLogRow::Aside {
            id: q.clone(),
            reason: "winter".into(),
            since: 1,
            title: "Find the heir".into()
        }],
        quest_log(&app, &w.campaign)
    );
    app.call(v1_3::abandon_quest, (q.clone(), s, "heir died".into()));
    assert_eq!(
        vec![v1_3::QuestLogRow::Dropped {
            closed: 1,
            id: q.clone(),
            reason: "heir died".into(),
            title: "Find the heir".into()
        }],
        quest_log(&app, &w.campaign)
    );
    let reopened = app.adhoc(&format!(
        "update x in quests.Abandoned where key x == {} set Shelved {{ taken_in: x.closed_in, reason: \"\" }}",
        key(&q)
    ));
    assert!(
        adhoc_violates(&reopened, "closed_quests_stay_closed"),
        "an abandoned quest joins the closed set: {reopened:?}"
    );
}

#[test]
fn release_1_3_calls_through_its_bundle_over_a_release_1_0_file() {
    let path = scratch("reopen");
    let old = App::at(path.clone(), 1, &v1_0::BUNDLE);
    let w = world_1_0(&old);
    old.call(
        v1_0::write_entity,
        (
            carry(w.warden.clone()),
            "Holds the pass".into(),
            "She keeps [[Greyhold]].".into(),
            String::new(),
            vec![carry(w.keep.clone())],
            Vec::new(),
        ),
    );
    old.call(
        v1_0::reveal,
        (carry(w.party.clone()), carry(w.keep.clone()), None),
    );
    old.db.close().expect("close");

    let app = App::at(path, 4, &v1_3::BUNDLE);
    let superseded = app.db.bundle(&v1_0::BUNDLE).map(drop);
    assert!(
        matches!(
            superseded,
            Err(v1_0::Error::Rejected {
                reason: RejectReason::BundleSuperseded { .. }
            })
        ),
        "the 1.3 open supersedes the 1.0 bundle: {superseded:?}"
    );
    let found = one(app.call(v1_3::lookup, (w.campaign.clone(), "Greyhold".into())));
    assert_eq!(
        (w.keep.clone(), "Settlement"),
        (found.id, found.kind.as_str()),
        "a town of 1.0 is a settlement"
    );
    assert_eq!(
        vec![v1_3::KnownEntitiesRow::Place {
            id: w.keep.clone(),
            name: "Greyhold".into(),
            summary: "A keep on the pass".into()
        }],
        all(app.call(v1_3::known_entities, (w.party.clone(),))),
        "a discovery of 1.0 survives"
    );
    let page = one(app.call(v1_3::entity_page, (w.warden.clone(),)));
    let mentions: Vec<&str> = page.mentions.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(vec!["Greyhold"], mentions);

    let map = app
        .call(
            v1_3::add_map,
            (
                w.campaign.clone(),
                "The pass".into(),
                "pass.png".into(),
                800,
                600,
            ),
        )
        .value;
    let pin = app
        .call(v1_3::pin_entity, (map.clone(), 120, 340, w.keep.clone()))
        .value;
    let off = app.try_call(
        v1_3::add_marker,
        (map.clone(), 900, 10, "Off the edge".into()),
    );
    assert!(violates(&off, "pin_inside_map"), "{off:?}");
    let view = one(app.call(v1_3::map_view, (map.clone(),)));
    assert_eq!(
        vec![v1_3::MapViewRowPins::OnEntity {
            entity: w.keep.clone(),
            id: pin,
            name: "Greyhold".into(),
            x: 120,
            y: 340
        }],
        view.pins
    );
    app.call(v1_3::reveal_map, (w.party.clone(), map.clone(), None));
    let known: Vec<_> = all(app.call(v1_3::known_maps, (w.party.clone(),)))
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(vec![map], known);

    let lantern = app
        .call(
            v1_3::add_item,
            (
                w.campaign.clone(),
                "Storm lantern".into(),
                String::new(),
                Some(w.warden.clone()),
            ),
        )
        .value;
    let found = one(app.call(v1_3::lookup, (w.campaign, "storm".into())));
    assert_eq!((lantern, "Item"), (found.id, found.kind.as_str()));
}

#[test]
fn release_1_2_opens() {
    open_db(scratch("opens-1-2"), 3, Vec::new());
}

#[test]
fn release_1_3_opens() {
    let app = Current::open("opens-1-3");
    assert!(all(app.call(v1_3::campaigns, ())).is_empty());
}

/// A world built at release 1.0, with a realm and a site under its keep, and a second
/// campaign, carried through release 1.1.
struct Located {
    w: World,
    gate: Key<Entities>,
    other: Key<Campaigns>,
}

/// Build a [`Located`] world at release 1.0 at `path`, then migrate the file to release 1.1.
fn located_over_release_1_1(path: PathBuf) -> Located {
    let app = App::at(path.clone(), 1, &v1_0::BUNDLE);
    let w = world_1_0(&app);
    let location = |name: &str, summary: &str, parent: Option<&Key<Entities>>, scale| {
        app.call(
            v1_0::add_location,
            (
                carry(w.campaign.clone()),
                name.into(),
                summary.into(),
                parent.map(|p| carry(p.clone())),
                scale,
            ),
        )
        .value
    };
    location("Vael", "The old empire", None, v1_0::Scale::Realm);
    let gate = location(
        "The Gate",
        "Under the keep",
        Some(&w.keep),
        v1_0::Scale::Spot,
    );
    let other = app
        .call(
            v1_0::new_campaign,
            ("Salt Roads".into(), "Caravans".into(), "The Drovers".into()),
        )
        .value;
    app.db.close().expect("close");
    migrate(path, 2);
    Located {
        w,
        gate: carry(gate),
        other: carry(other),
    }
}

/// Returns the names of the rows of `scope`, sorted.
fn names(db: &Database, scope: &str) -> Vec<String> {
    let mut out = adhoc(db, &format!("from e in {scope} select e.name"), &[])
        .unwrap_or_else(|e| panic!("{scope}: {e:?}"));
    out.sort();
    out
}

#[test]
fn release_1_2_routes_locations_by_scale_over_release_1_1() {
    let path = scratch("split");
    let Located { w, gate, .. } = located_over_release_1_1(path.clone());
    let db = open_db(path, 3, Vec::new());
    assert_eq!(vec![text("Vael")], names(&db, "entities.Region"));
    assert_eq!(vec![text("Greyhold")], names(&db, "entities.Settlement"));
    assert_eq!(vec![text("The Gate")], names(&db, "entities.Site"));
    let parent = adhoc(
        &db,
        &format!(
            "from s in entities.Site where key s == {} select s.parent",
            key(&gate)
        ),
        &[],
    );
    assert_eq!(
        vec![some(&key(&w.keep))],
        parent.expect("the site's parent")
    );
    let location = adhoc(
        &db,
        &format!(
            "from n in entities.Npc where key n == {} select n.location",
            key(&w.warden)
        ),
        &[],
    );
    assert_eq!(
        vec![some(&key(&w.keep))],
        location.expect("the warden's location")
    );
}

#[test]
fn release_1_3_routes_locations_by_scale_over_release_1_1() {
    let path = scratch("split-1-3");
    let Located { w, gate, .. } = located_over_release_1_1(path.clone());
    migrate(path.clone(), 3);
    let app = App::at(path, 4, &v1_3::BUNDLE);
    let kinds = |q: &str| -> Vec<(String, String)> {
        all(app.call(v1_3::lookup, (w.campaign.clone(), q.into())))
            .into_iter()
            .map(|r| (r.name, r.kind))
            .collect()
    };
    assert_eq!(vec![("Vael".into(), "Region".into())], kinds("Vael"));
    assert_eq!(
        vec![("Greyhold".into(), "Settlement".into())],
        kinds("Greyhold")
    );
    assert_eq!(vec![("The Gate".into(), "Site".into())], kinds("Gate"));
    let page = one(app.call(v1_3::entity_page, (gate,)));
    assert_eq!("Site", page.kind);
}

#[test]
fn a_split_location_and_its_parent_stay_in_one_campaign() {
    let path = scratch("split-parent");
    let Located { w, other, .. } = located_over_release_1_1(path.clone());
    let db = open_db(path, 3, Vec::new());
    let stray = adhoc(
        &db,
        &format!(
            "insert entities.Site {{ campaign: {}, name: \"Waystone\", parent: parent }}",
            key(&other)
        ),
        &[(
            "parent",
            "Option (Key entities.(Region | Settlement | Site))",
            &some(&key(&w.keep)),
        )],
    );
    assert!(
        adhoc_violates(&stray, "location_parent_in_campaign"),
        "{stray:?}"
    );
    let moved = adhoc(
        &db,
        &format!(
            "update x in entities.Settlement where key x == {} set Settlement {{ x with campaign: {} }}",
            key(&w.keep),
            key(&other)
        ),
        &[],
    );
    assert!(
        adhoc_violates(&moved, "location_parent_in_campaign"),
        "the gate under the keep stays in the first campaign: {moved:?}"
    );
}
