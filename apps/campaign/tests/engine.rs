//! The schema opened in the engine, with the bundle's queries and transactions run as ad hoc
//! text through the in-process API.

use std::path::PathBuf;

use adtb::{
    CallOptions, CreateOptions, Database, Log, OpenOptions, TxnCallOptions, TxnOptions, Value,
};
use adtb_lang::ast::{BundleFile, BundleItem, Param};
use adtb_lang::check::{Checker, Context};
use adtb_lang::evolve::Source;
use adtb_lang::parse::{Options, parse_bundle, parse_migration, parse_type};
use adtb_lang::resolve::Visible;
use adtb_lang::textual::{Form, parse, print};
use adtb_lang::types::Ty;
use campaign::{BUNDLE, MIGRATIONS};

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

/// The constraints of migration 0001 the engine does not admit yet.
const UNADMITTED: [&str; 2] = ["location_parent_in_campaign", "quest_giver_in_campaign"];

/// Returns the first `n` migrations, with [`UNADMITTED`] left out of migration 0001.
fn admitted(n: usize) -> Vec<String> {
    let mut log: Vec<String> = MIGRATIONS[..n].iter().map(|m| (*m).to_string()).collect();
    for name in UNADMITTED {
        let first = &mut log[0];
        let start = first
            .find(&format!("add constraint {name} ="))
            .unwrap_or_else(|| panic!("no constraint {name}"));
        let end = first[start..]
            .find("\n\n")
            .map_or(first.len(), |i| start + i + 2);
        first.replace_range(start..end, "");
    }
    log
}

struct App {
    db: Database,
    bundle: BundleFile,
}

impl App {
    /// Open a new database under `name` at the first `n` migrations.
    fn open(name: &str, n: usize) -> App {
        App::at(scratch(name), n)
    }

    /// Open or create the database at `path` at the first `n` admitted migrations.
    fn at(path: PathBuf, n: usize) -> App {
        App::with(path, &Log::new(admitted(n)))
    }

    fn with(path: PathBuf, log: &Log) -> App {
        let options = OpenOptions {
            create: Some(CreateOptions::default()),
            workers: Some(2),
            ..OpenOptions::default()
        };
        let db = adtb::open(path, log, &options).unwrap_or_else(|e| panic!("open: {e:?}"));
        let bundle = parse_bundle(BUNDLE, Options::default())
            .ok()
            .expect("the bundle parses");
        App { db, bundle }
    }

    /// Returns the body text of the bundle's query or transaction `name` and its parameters.
    fn item(&self, name: &str) -> (&str, &[Param]) {
        let found = self.bundle.items.iter().find_map(|i| match i {
            BundleItem::Query(q) if q.name.text == name => Some((q.body.span, &q.params[..])),
            BundleItem::Txn(t) if t.name.text == name => Some((t.body.span, &t.params[..])),
            _ => None,
        });
        let (span, params) = found.unwrap_or_else(|| panic!("no `{name}` in the bundle"));
        (&BUNDLE[span.range()], params)
    }

    fn params(&self, params: &[Param], args: &[&str]) -> Vec<(String, Ty, Value)> {
        assert_eq!(params.len(), args.len(), "arguments");
        let schema = self.db.schema();
        let c = Checker::new(schema, Visible::all(schema), Context::Adhoc);
        params
            .iter()
            .zip(args)
            .map(|(p, a)| {
                let ty = c
                    .resolve_type(&p.ty)
                    .unwrap_or_else(|d| panic!("`{}`: {}", p.name.text, d.message));
                let v = parse(a, &Form::of(schema, &ty))
                    .unwrap_or_else(|e| panic!("`{}` = `{a}`: {e}", p.name.text));
                (p.name.text.clone(), ty, v)
            })
            .collect()
    }

    /// Runs the bundle's `name` as ad hoc text, each argument written in the textual syntax,
    /// and returns every row of its results, printed.
    fn call(&self, name: &str, args: &[&str]) -> Result<Vec<String>, adtb::Error> {
        let (text, params) = self.item(name);
        let bound = self.params(params, args);
        let refs: Vec<(&str, Ty, Value)> = bound
            .iter()
            .map(|(n, t, v)| (n.as_str(), t.clone(), v.clone()))
            .collect();
        let out = self.db.query_text(text, &refs, &CallOptions::default())?;
        Ok(printed(out))
    }

    fn ok(&self, name: &str, args: &[&str]) -> Vec<String> {
        self.call(name, args)
            .unwrap_or_else(|e| panic!("`{name}` fails: {e:?}"))
    }

    /// Returns the one row `name` returns.
    fn one(&self, name: &str, args: &[&str]) -> String {
        let mut rows = self.ok(name, args);
        assert_eq!(1, rows.len(), "`{name}` returns one row: {rows:?}");
        rows.remove(0)
    }

    /// Returns the rows of ad hoc `text`, each parameter given as its name, type and value in
    /// the textual syntax.
    fn adhoc_with(
        &self,
        text: &str,
        given: &[(&str, &str, &str)],
    ) -> Result<Vec<String>, adtb::Error> {
        let schema = self.db.schema();
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
        Ok(printed(self.db.query_text(
            text,
            &params,
            &CallOptions::default(),
        )?))
    }

    /// Returns the rows of ad hoc `text`.
    fn adhoc(&self, text: &str) -> Result<Vec<String>, adtb::Error> {
        Ok(printed(self.db.query_text(
            text,
            &[],
            &CallOptions::default(),
        )?))
    }
}

fn printed(results: Vec<adtb::DynRows>) -> Vec<String> {
    results
        .into_iter()
        .flat_map(|rows| {
            let form = rows.form().clone();
            rows.map(move |r| print(&r.expect("a row"), &form).expect("a row prints"))
        })
        .collect()
}

fn text(t: &str) -> String {
    format!("{t:?}")
}

fn some(v: &str) -> String {
    format!("Some {{ value: {v} }}")
}

/// A campaign with one party, a location and an NPC there.
struct World {
    campaign: String,
    party: String,
    keep: String,
    warden: String,
}

fn world(app: &App) -> World {
    let campaign = app.one(
        "new_campaign",
        &[
            &text("Ashes of Vael"),
            &text("A fallen empire"),
            &text("The Lanterns"),
        ],
    );
    let party = app.one("lookup", &[&campaign, &text("Lanterns")]);
    let party = field(&party, "id");
    let keep = app.one(
        "add_location",
        &[
            &campaign,
            &text("Greyhold"),
            &text("A keep on the pass"),
            "None",
            "Town",
        ],
    );
    let warden = app.one(
        "add_npc",
        &[
            &campaign,
            &text("Warden Ilsa"),
            &text("Holds the pass"),
            &some(&keep),
        ],
    );
    World {
        campaign,
        party,
        keep,
        warden,
    }
}

/// Returns the printed value of field `name` of a printed record.
fn field(record: &str, name: &str) -> String {
    let at = record
        .find(&format!("{name}: "))
        .unwrap_or_else(|| panic!("no `{name}` in {record}"));
    let rest = &record[at + name.len() + 2..];
    let mut depth = 0;
    let end = rest
        .char_indices()
        .find(|&(_, c)| {
            match c {
                '(' | '{' | '[' => depth += 1,
                ')' | '}' | ']' if depth > 0 => depth -= 1,
                ',' | '}' if depth == 0 => return true,
                _ => {}
            }
            false
        })
        .map_or(rest.len(), |(i, _)| i);
    rest[..end].trim().to_string()
}

#[test]
fn release_1_0_opens() {
    App::with(scratch("opens"), &Log::new(MIGRATIONS[..1].iter().copied()));
}

#[test]
fn release_1_1_opens_over_release_1_0() {
    let path = scratch("upgrade");
    let app = App::at(path.clone(), 1);
    let w = world(&app);
    app.db.close().expect("close");
    let app = App::at(path, 2);
    let found = app.ok("lookup", &[&w.campaign, &text("ilsa")]);
    assert_eq!(1, found.len(), "{found:?}");
    let map = app.adhoc_with(
        &format!(
        r#"transact
             let m = insert maps {{ campaign: {c}, title: "The pass", image: "pass.png", width: 800, height: 600 }}
             insert pins.OfEntity {{ map: key m, x: 120, y: 340, entity: {keep} }}
             insert discoveries.KnowsMap {{ party: party, learned_in: None, map: key m }}
             return key m"#,
            c = w.campaign,
            keep = w.keep,
        ),
        &[("party", "Key entities.Party", &w.party)],
    );
    assert!(map.is_ok(), "{map:?}");
    let off = app.adhoc(&format!(
        "insert pins.Marker {{ map: {}, x: 900, y: 10, label: \"Off the edge\" }}",
        map.expect("a map")[0]
    ));
    assert!(format!("{off:?}").contains("pin_inside_map"), "{off:?}");
}

#[test]
fn the_bundle_pins_version_1_by_its_hash() {
    let text = MIGRATIONS[0];
    let file = parse_migration(text, Options::default())
        .ok()
        .expect("parses");
    let v1 = Source::default().migrate(&file, text).expect("applies");
    let schema = v1.schema().expect("builds");
    let hash: String = adtb::catalog::version_hash(&schema, &v1)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let bundle = parse_bundle(BUNDLE, Options::default())
        .ok()
        .expect("parses");
    assert_eq!(1, bundle.header.pins.number);
    assert_eq!(Some(hash), bundle.header.pins.hash);
}

#[test]
fn prep_builds_a_world_the_gm_can_look_up() {
    let app = App::open("prep", 1);
    let w = world(&app);
    let found = app.ok("lookup", &[&w.campaign, &text("ilsa")]);
    assert_eq!(1, found.len(), "{found:?}");
    assert!(found[0].contains("Warden Ilsa"), "{found:?}");
    app.ok(
        "write_entity",
        &[
            &w.warden,
            &text("Holds the pass"),
            &text("She keeps [[Greyhold]]."),
            &text("Sold the pass to [[The Lanterns]]."),
            &format!("[{}]", w.keep),
            &format!("[{}]", w.party),
        ],
    );
    let page = app.one("entity_page", &[&w.warden]);
    assert!(page.contains("Greyhold"), "{page}");
    let keep = app.one("entity_page", &[&w.keep]);
    assert!(
        keep.contains("InEntity"),
        "the keep is mentioned by the warden: {keep}"
    );
}

#[test]
fn players_see_only_what_their_party_discovered() {
    let app = App::open("discovery", 1);
    let w = world(&app);
    app.ok(
        "write_entity",
        &[
            &w.warden,
            &text("Holds the pass"),
            &text("She keeps [[Greyhold]]."),
            &text("Sold the pass."),
            &format!("[{}]", w.keep),
            "[]",
        ],
    );
    assert!(app.ok("known_entities", &[&w.party]).is_empty());
    app.ok("reveal", &[&w.party, &w.warden, "None"]);
    app.ok("reveal", &[&w.party, &w.warden, "None"]);
    let known = app.ok("known_entities", &[&w.party]);
    assert_eq!(1, known.len(), "{known:?}");
    let page = app.one("known_entity", &[&w.party, &w.warden]);
    assert!(
        !page.contains("Sold the pass"),
        "secrets stay hidden: {page}"
    );
    assert_eq!(
        "[]",
        field(&page, "mentions"),
        "an undiscovered link stays hidden: {page}"
    );
    app.ok("reveal", &[&w.party, &w.keep, "None"]);
    let page = app.one("known_entity", &[&w.party, &w.warden]);
    assert!(
        field(&page, "mentions").contains("Greyhold"),
        "a discovered link shows: {page}"
    );
}

#[test]
fn a_discovery_never_crosses_campaigns() {
    let app = App::open("crossing", 1);
    let w = world(&app);
    let other = app.one(
        "new_campaign",
        &[&text("Other"), &text(""), &text("The Others")],
    );
    let stranger = app.one("add_npc", &[&other, &text("Stranger"), &text(""), "None"]);
    let refused = app.call("reveal", &[&w.party, &stranger, "None"]);
    assert!(
        format!("{refused:?}").contains("discovery_in_campaign"),
        "{refused:?}"
    );
}

/// Plans and starts session 1 of `w`, returning its key.
fn live_session(app: &App, w: &World) -> String {
    let s = app.one("plan_session", &[&w.campaign, &w.party, &text("The pass")]);
    app.ok("start_session", &[&s]);
    s
}

#[test]
fn an_encounter_runs_round_by_round() {
    let app = App::open("encounter", 1);
    let w = world(&app);
    let s = live_session(&app, &w);
    let e = app.one(
        "add_encounter",
        &[&s, &text("Ambush"), &text("Bandits on the road")],
    );
    let ilsa = app.one("add_cast", &[&e, &w.warden, &text("Ilsa")]);
    let bandit = app.one("add_extra", &[&e, &text("Bandit"), &text("bandit")]);
    app.ok(
        "start_encounter",
        &[
            &e,
            &format!(
                "[{{ combatant: {ilsa}, roll: 17, slot: 1 }}, {{ combatant: {bandit}, roll: 9, slot: 2 }}]"
            ),
        ],
    );
    let t = app.one("tracker", &[&e]);
    assert_eq!("1", field(&t, "round"), "{t}");
    assert_eq!("1", field(&t, "turn"), "{t}");
    app.ok("next_turn", &[&e]);
    assert_eq!("2", field(&app.one("tracker", &[&e]), "turn"));
    app.ok("next_turn", &[&e]);
    let t = app.one("tracker", &[&e]);
    assert_eq!(
        ("2", "1"),
        (field(&t, "round").as_str(), field(&t, "turn").as_str()),
        "{t}"
    );
    let wolf = app.one("add_extra", &[&e, &text("Wolf"), &text("wolf")]);
    app.ok("join_fight", &[&e, &wolf, "4", "3"]);
    app.ok("leave_fight", &[&ilsa]);
    let t = app.one("tracker", &[&e]);
    assert_eq!(
        ("2", "2"),
        (field(&t, "round").as_str(), field(&t, "turn").as_str()),
        "the acting slot left: {t}"
    );
    let seen = app.one("party_encounter", &[&w.party]);
    assert!(
        seen.contains("Bandit") && !seen.contains("bandit\""),
        "{seen}"
    );
    app.ok("end_encounter", &[&e, &text("Bandits fled")]);
    assert!(app.ok("tracker", &[&e]).is_empty());
}

#[test]
fn a_second_live_session_is_refused() {
    let app = App::open("two-live", 1);
    let w = world(&app);
    live_session(&app, &w);
    let s2 = app.one("plan_session", &[&w.campaign, &w.party, &text("Again")]);
    let refused = app.call("start_session", &[&s2]);
    assert!(refused.is_err(), "{refused:?}");
}

#[test]
fn the_dead_do_not_fight_in_later_sessions() {
    let app = App::open("dead", 1);
    let w = world(&app);
    let s1 = live_session(&app, &w);
    let npc = w.warden.clone();
    app.ok("kill", &[&npc, &s1]);
    app.ok(
        "end_session",
        &[&s1, "2026-10-24", &text("Ilsa fell."), &text("Ilsa fell.")],
    );
    let s2 = live_session(&app, &w);
    let e = app.one("add_encounter", &[&s2, &text("Haunting"), &text("")]);
    let refused = app.call("add_cast", &[&e, &npc, &text("Ilsa")]);
    assert!(
        format!("{refused:?}").contains("the_dead_stay_dead"),
        "{refused:?}"
    );
    let known = app.ok("recaps", &[&w.party]);
    assert_eq!(1, known.len(), "{known:?}");
}

#[test]
fn a_closed_quest_stays_closed() {
    let app = App::open("quest", 1);
    let w = world(&app);
    let s = live_session(&app, &w);
    let q = app.one(
        "add_quest",
        &[
            &w.campaign,
            &text("Clear the pass"),
            &text(""),
            &some(&w.warden),
            "None",
        ],
    );
    app.ok("take_quest", &[&q, &s]);
    app.ok("close_quest", &[&q, &s, "False", &text("The pass fell")]);
    let log = app.one("quest_log", &[&w.campaign]);
    assert!(log.starts_with("Done"), "{log}");
    let reopened = app.adhoc(&format!(
        "update x in quests.Failed where key x == {q} set Active {{ taken_in: x.closed_in }}"
    ));
    assert!(
        format!("{reopened:?}").contains("closed_quests_stay_closed"),
        "{reopened:?}"
    );
}

#[test]
fn two_gms_planning_the_same_session_number_do_not_both_commit() {
    let app = App::open("collide", 1);
    let w = world(&app);
    let (text_, params) = app.item("plan_session");
    let bound = app.params(params, &[&w.campaign, &w.party, &text("Concurrent")]);
    let refs: Vec<(&str, Ty, Value)> = bound
        .iter()
        .map(|(n, t, v)| (n.as_str(), t.clone(), v.clone()))
        .collect();
    let mut a = app.db.begin(&TxnOptions::default()).expect("begin a");
    a.query_text(text_, &refs, &TxnCallOptions::default())
        .expect("a plans");
    let (a_done, b_done) = std::thread::scope(|s| {
        let (begun, wait) = std::sync::mpsc::channel();
        let db = &app.db;
        let b = s.spawn(move || {
            let mut b = db.begin(&TxnOptions::default())?;
            begun.send(()).expect("the test waits");
            b.query_text(text_, &refs, &TxnCallOptions::default())?;
            b.commit(None)
        });
        wait.recv().expect("b begins");
        let a_done = a.commit(None);
        (a_done, b.join().expect("b runs"))
    });
    assert!(a_done.is_ok(), "{a_done:?}");
    assert!(
        b_done.as_ref().is_err_and(adtb::Error::is_serialization),
        "the second plan of session 1 fails to serialize: {b_done:?}"
    );
    let numbers = app
        .adhoc(&format!(
            "from s in sessions where s.campaign == {} select s.number",
            w.campaign
        ))
        .expect("the sessions");
    assert_eq!(vec!["1"], numbers);
}

#[test]
fn release_1_3_shelves_a_quest_over_release_1_1() {
    let mut log = admitted(2);
    log.push(MIGRATIONS[3].to_string());
    let app = App::with(scratch("shelve"), &Log::new(log));
    let w = world(&app);
    let s = live_session(&app, &w);
    let q = app.one(
        "add_quest",
        &[
            &w.campaign,
            &text("Find the heir"),
            &text(""),
            "None",
            "None",
        ],
    );
    app.ok("take_quest", &[&q, &s]);
    let shelved = app.adhoc(&format!(
        "update x in quests.Active where key x == {q} set Shelved {{ taken_in: x.taken_in, reason: \"winter\" }}"
    ));
    assert!(shelved.is_ok(), "{shelved:?}");
    let abandoned = app.adhoc(&format!(
        "update x in quests.Shelved where key x == {q} set Abandoned {{ closed_in: x.taken_in, reason: \"heir died\" }}"
    ));
    assert!(abandoned.is_ok(), "{abandoned:?}");
    let reopened = app.adhoc(&format!(
        "update x in quests.Abandoned where key x == {q} set Shelved {{ taken_in: x.closed_in, reason: \"\" }}"
    ));
    assert!(
        format!("{reopened:?}").contains("closed_quests_stay_closed"),
        "an abandoned quest joins the closed set: {reopened:?}"
    );
}
