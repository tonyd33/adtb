use std::collections::{BTreeMap, BTreeSet};
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use adtb::runtime::{Entropy, Threads};
use adtb::storage::{Damage, Edit, Finding, HEADER_LEN, OpenError, Options, PageKind, Pager, Pid};
use adtb_sim::{
    Config, Fate, Faults, FindingKind, Ledger, Outcome, Recovered, Sim, SimRuntime, Violation,
    check,
};

const PAGE: usize = 4096;
const SLOT: usize = 16;
const SLOTS: usize = (PAGE - HEADER_LEN) / SLOT;
const PAGES: usize = 24;
const NAME: &str = "app.adt";
const FATAL: &str = "fatal I/O error";

/// How the database is laid out on the simulated device.
#[derive(Clone, Copy, Debug)]
struct Layout {
    atomic: bool,
    log_block: u32,
    /// Worker threads sharing one pager; 0 runs the single-threaded workload.
    threads: usize,
    /// Pool frames; more than the pages every thread may latch at once.
    frames: usize,
}

const PLAIN: Layout = Layout {
    atomic: false,
    log_block: 512,
    threads: 0,
    frames: 5,
};

fn options(layout: Layout) -> Options {
    let Layout {
        atomic, log_block, ..
    } = layout;
    Options {
        page_size: PAGE as u32,
        log_block,
        segment_size: 64 * 1024,
        pool_frames: layout.frames,
        max_pages: 4096,
        atomic_page_writes: atomic,
        ..Options::new(NAME)
    }
}

fn quiet() -> Faults {
    Faults {
        lose_unsynced: true,
        delay_completions: true,
        ..Faults::default()
    }
}

fn value(xid: u64, pid: Pid, slot: usize) -> u64 {
    let mut h = xid
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(pid.0 << 20)
        .wrapping_add(slot as u64);
    h ^= h >> 29;
    h.wrapping_mul(0xbf58_476d_1ce4_e5b9) | 1
}

/// What the client knows, kept outside the simulated process so it survives crashes.
#[derive(Default)]
struct World {
    pages: Vec<Pid>,
    next_xid: u64,
    /// The slots each transaction wrote, by page.
    writes: BTreeMap<u64, Vec<(Pid, Vec<usize>)>>,
    /// The recorded-event count when each transaction was submitted and acknowledged.
    submitted: BTreeMap<u64, usize>,
    acked: BTreeMap<u64, usize>,
}

#[derive(Clone)]
struct Client {
    ledger: Ledger,
    world: Arc<Mutex<World>>,
}

impl Client {
    fn new() -> Client {
        Client {
            ledger: Ledger::new(),
            world: Arc::new(Mutex::new(World::default())),
        }
    }
}

fn silence_fatal_panics() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let default = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let msg = info
                .payload()
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| info.payload().downcast_ref::<&str>().copied())
                .unwrap_or("");
            if !msg.starts_with(FATAL) {
                default(info);
            }
        }));
    });
}

/// Run `body` as a process, treating a crash caused by a failed write or force as a crash.
fn run_process<F: FnOnce(SimRuntime)>(sim: &mut Sim, body: F) -> Outcome {
    match panic::catch_unwind(AssertUnwindSafe(|| sim.run(body))) {
        Ok(o) => o,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_default();
            if msg.starts_with(FATAL) {
                Outcome::Crashed
            } else {
                panic::resume_unwind(payload)
            }
        }
    }
}

fn setup(sim: &mut Sim, client: &Client, layout: Layout) {
    let world = client.world.clone();
    let outcome = sim.run(move |rt| {
        let pager = Pager::create(rt, &options(layout)).expect("create");
        let mut pages = Vec::new();
        for _ in 0..PAGES {
            pages.push(
                pager
                    .allocate(1, PageKind::HeapSlotted, 7)
                    .expect("allocate"),
            );
        }
        pager.flush_log();
        world.lock().unwrap().pages = pages;
    });
    assert_eq!(Outcome::Finished, outcome);
}

/// Open the database and run random transactions, flushes and checkpoints for `steps` steps.
fn workload(rt: SimRuntime, client: &Client, layout: Layout, steps: usize) {
    let Ok((mut pager, _)) = Pager::open(rt.clone(), &options(layout)) else {
        return;
    };
    let pages = client.world.lock().unwrap().pages.clone();
    let mut unacked: Vec<u64> = Vec::new();
    let ack = |pager: &mut Pager<SimRuntime>, unacked: &mut Vec<u64>| {
        pager.flush_log();
        let stamp = rt.recorded();
        let mut w = client.world.lock().unwrap();
        for xid in unacked.drain(..) {
            client.ledger.acknowledge(xid);
            w.acked.insert(xid, stamp);
        }
    };
    for _ in 0..steps {
        match rt.next_u64() % 20 {
            0..=13 => {
                let n = 1 + (rt.next_u64() % 3) as usize;
                let mut chosen = BTreeSet::new();
                while chosen.len() < n {
                    chosen.insert(pages[(rt.next_u64() % pages.len() as u64) as usize]);
                }
                let xid = {
                    let mut w = client.world.lock().unwrap();
                    w.next_xid += 1;
                    w.next_xid
                };
                let mut writes = Vec::new();
                let mut changes = Vec::new();
                for pid in chosen {
                    let start = (rt.next_u64() % SLOTS as u64) as usize;
                    let len = 1 + (rt.next_u64() % 6) as usize;
                    let slots: Vec<usize> = (start..(start + len).min(SLOTS)).collect();
                    let mut bytes = Vec::new();
                    for &s in &slots {
                        bytes.extend_from_slice(&xid.to_le_bytes());
                        bytes.extend_from_slice(&value(xid, pid, s).to_le_bytes());
                    }
                    changes.push((
                        pid,
                        vec![Edit {
                            offset: (HEADER_LEN + start * SLOT) as u32,
                            bytes,
                        }],
                    ));
                    writes.push((pid, slots));
                }
                {
                    let mut w = client.world.lock().unwrap();
                    w.writes.insert(xid, writes);
                    w.submitted.insert(xid, rt.recorded());
                }
                client.ledger.submit(xid);
                let pids: Vec<Pid> = changes.iter().map(|(p, _)| *p).collect();
                let committed = pager.commit(xid, &pids, |pages| {
                    for (page, (_, edits)) in pages.iter_mut().zip(&changes) {
                        for e in edits {
                            let at = e.offset as usize;
                            page[at..at + e.bytes.len()].copy_from_slice(&e.bytes);
                        }
                    }
                });
                match committed {
                    Ok(_) => unacked.push(xid),
                    Err(_) => client.ledger.abort(xid),
                }
                if rt.next_u64().is_multiple_of(2) {
                    ack(&mut pager, &mut unacked);
                }
            }
            14..=16 => {
                let n = 1 + (rt.next_u64() % 4) as usize;
                pager.flush_pages(n);
            }
            17 => pager.checkpoint(),
            _ => ack(&mut pager, &mut unacked),
        }
    }
    ack(&mut pager, &mut unacked);
}

/// Returns whether every slot of `page` holds a value some transaction could have written.
fn plausible(pid: Pid, page: &[u8]) -> bool {
    (0..SLOTS).all(|s| {
        let at = HEADER_LEN + s * SLOT;
        let xid = u64::from_le_bytes(page[at..at + 8].try_into().unwrap());
        let v = u64::from_le_bytes(page[at + 8..at + 16].try_into().unwrap());
        (xid == 0 && v == 0) || v == value(xid, pid, s)
    })
}

/// One of several threads sharing `pager`: transactions under exclusive latches taken in page
/// order, optimistic and shared reads, flushes and checkpoints.
fn worker(pager: &Pager<SimRuntime>, rt: &SimRuntime, client: &Client, steps: usize) {
    let pages = client.world.lock().unwrap().pages.clone();
    let pick = || pages[(rt.next_u64() % pages.len() as u64) as usize];
    let mut unacked: Vec<u64> = Vec::new();
    let ack = |unacked: &mut Vec<u64>| {
        pager.flush_log();
        let stamp = rt.recorded();
        let mut w = client.world.lock().unwrap();
        for xid in unacked.drain(..) {
            client.ledger.acknowledge(xid);
            w.acked.insert(xid, stamp);
        }
    };
    for _ in 0..steps {
        match rt.next_u64() % 12 {
            0..=6 => {
                let n = 1 + (rt.next_u64() % 3) as usize;
                let mut chosen = BTreeSet::new();
                while chosen.len() < n {
                    chosen.insert(pick());
                }
                let Ok(mut guards) = chosen
                    .iter()
                    .map(|&p| pager.exclusive(p))
                    .collect::<Result<Vec<_>, _>>()
                else {
                    continue;
                };
                let mut writes = Vec::new();
                for &pid in &chosen {
                    let start = (rt.next_u64() % SLOTS as u64) as usize;
                    let len = 1 + (rt.next_u64() % 6) as usize;
                    writes.push((pid, (start..(start + len).min(SLOTS)).collect::<Vec<_>>()));
                }
                let xid = {
                    let mut w = client.world.lock().unwrap();
                    w.next_xid += 1;
                    let xid = w.next_xid;
                    w.writes.insert(xid, writes.clone());
                    w.submitted.insert(xid, rt.recorded());
                    xid
                };
                client.ledger.submit(xid);
                let mut refs: Vec<_> = guards.iter_mut().collect();
                pager.commit_all(xid, &mut refs, |copies| {
                    for (page, (pid, slots)) in copies.iter_mut().zip(&writes) {
                        for &s in slots {
                            let at = HEADER_LEN + s * SLOT;
                            page[at..at + 8].copy_from_slice(&xid.to_le_bytes());
                            page[at + 8..at + 16]
                                .copy_from_slice(&value(xid, *pid, s).to_le_bytes());
                        }
                    }
                });
                drop(guards);
                unacked.push(xid);
                if rt.next_u64().is_multiple_of(3) {
                    ack(&mut unacked);
                }
            }
            7 | 8 => {
                let pid = pick();
                while let Ok(o) = pager.optimistic(pid) {
                    let ok = o.read(|p| plausible(pid, p));
                    if o.validate() {
                        assert!(ok, "a validated optimistic read of {pid:?} saw a torn page");
                        break;
                    }
                    rt.yield_now();
                }
            }
            9 => {
                if let Ok(p) = pager.shared(pick()) {
                    assert!(plausible(p.pid(), &p), "a shared latch saw a torn page");
                }
            }
            10 => {
                pager.flush_pages(2);
            }
            _ if rt.next_u64().is_multiple_of(4) => pager.checkpoint(),
            _ => ack(&mut unacked),
        }
    }
    ack(&mut unacked);
}

/// Open the database and run `layout.threads` workers over it for `steps` steps each.
fn concurrent(rt: SimRuntime, client: &Client, layout: Layout, steps: usize) {
    let Ok((pager, _)) = Pager::open(rt.clone(), &options(layout)) else {
        return;
    };
    let pager = Arc::new(pager);
    let workers: Vec<_> = (0..layout.threads)
        .map(|i| {
            let (pager, rt2, client) = (pager.clone(), rt.clone(), client.clone());
            rt.spawn(&format!("worker {i}"), move || {
                worker(&pager, &rt2, &client, steps)
            })
        })
        .collect();
    for w in workers {
        rt.join(w);
    }
}

/// What a reopened database shows the checks.
#[derive(Debug, Default)]
struct Report {
    open: Option<OpenError>,
    findings: Vec<Finding>,
    commits: BTreeSet<u64>,
    pages: BTreeMap<Pid, Option<Vec<u8>>>,
}

impl Recovered for Report {
    fn committed(&self) -> BTreeSet<u64> {
        self.commits.clone()
    }

    fn verify(&self) -> Vec<adtb_sim::Finding> {
        self.findings
            .iter()
            .map(|f| adtb_sim::Finding {
                kind: FindingKind::Structure,
                at: format!("{f:?}"),
            })
            .collect()
    }
}

fn inspect(rt: SimRuntime, pages: &[Pid], layout: Layout) -> Report {
    let (pager, recovery) = match Pager::open(rt, &options(layout)) {
        Ok(opened) => opened,
        Err(e) => {
            return Report {
                open: Some(e),
                ..Report::default()
            };
        }
    };
    let v = pager.verify();
    let mut findings = v.findings;
    findings.extend(recovery.log.into_iter().map(Finding::Log));
    findings.sort();
    findings.dedup();
    let mut report = Report {
        open: None,
        findings,
        commits: v.commits.into_iter().collect(),
        pages: BTreeMap::new(),
    };
    for &pid in pages {
        report
            .pages
            .insert(pid, pager.read(pid).ok().map(|p| p.to_vec()));
    }
    report
}

/// Returns the transaction each slot of `page` shows, failing on a slot no transaction wrote.
fn slots(pid: Pid, page: &[u8], world: &World) -> Result<Vec<u64>, String> {
    let mut out = Vec::new();
    for s in 0..SLOTS {
        let at = HEADER_LEN + s * SLOT;
        let xid = u64::from_le_bytes(page[at..at + 8].try_into().unwrap());
        let v = u64::from_le_bytes(page[at + 8..at + 16].try_into().unwrap());
        let wrote = world
            .writes
            .get(&xid)
            .is_some_and(|ws| ws.iter().any(|(p, ss)| *p == pid && ss.contains(&s)));
        let ok = if xid == 0 {
            v == 0
        } else {
            wrote && v == value(xid, pid, s)
        };
        if !ok {
            return Err(format!("{pid:?} slot {s} holds ({xid}, {v:#x})"));
        }
        out.push(xid);
    }
    Ok(out)
}

/// Check what `report` shows against the client's records, given the fates of `ledger`, and
/// returns the violations that remain once those the reported damage explains are dropped.
fn judge(
    report: &Report,
    ledger: &Ledger,
    world: &World,
    trace: &str,
    damage_allowed: bool,
) -> Vec<String> {
    let mut problems = Vec::new();
    if let Some(e) = report.open {
        if !damage_allowed {
            problems.push(format!("open failed: {e:?}"));
        }
        return problems;
    }
    let log_trouble = report.findings.iter().any(|f| matches!(f, Finding::Log(_)))
        || trace
            .lines()
            .any(|l| l.starts_with("fault lost write") && l.contains("-log/"));
    for v in check(ledger, report) {
        match v {
            Violation::Lost(_) if log_trouble => {}
            Violation::Found(_) if damage_allowed => {}
            v => problems.push(format!("{v:?}")),
        }
    }
    for (pid, page) in &report.pages {
        let Some(page) = page else {
            let reported = report.findings.iter().any(|f| match f {
                Finding::Page { pid: p, .. } => p == pid,
                _ => false,
            });
            if !reported {
                problems.push(format!("{pid:?} refused without a finding"));
            }
            continue;
        };
        let shown = match slots(*pid, page, world) {
            Ok(s) => s,
            Err(e) => {
                problems.push(e);
                continue;
            }
        };
        for (s, &x) in shown.iter().enumerate() {
            if x != 0 && !log_trouble && !report.commits.contains(&x) {
                problems.push(format!("{pid:?} slot {s} shows {x}, which never committed"));
            }
            if x != 0 && ledger.fate(x) == Some(Fate::Aborted) {
                problems.push(format!("{pid:?} slot {s} shows aborted {x}"));
            }
            let newer_acked = world.writes.iter().find(|(w, ws)| {
                **w > x
                    && report.commits.contains(w)
                    && ledger.fate(**w) == Some(Fate::Acknowledged)
                    && ws.iter().any(|(p, ss)| p == pid && ss.contains(&s))
            });
            if let Some((w, _)) = newer_acked {
                problems.push(format!(
                    "{pid:?} slot {s} shows {x}, losing acknowledged {w}"
                ));
            }
        }
    }
    problems
}

/// Settle every commit left in doubt by what the reopened database shows.
fn resolve(report: &Report, ledger: &Ledger, world: &World) {
    let visible: BTreeSet<u64> = report
        .pages
        .iter()
        .filter_map(|(pid, p)| p.as_ref().map(|p| (pid, p)))
        .filter_map(|(pid, p)| slots(*pid, p, world).ok())
        .flatten()
        .collect();
    for &xid in world.writes.keys() {
        if ledger.fate(xid) == Some(Fate::InDoubt) {
            if report.commits.contains(&xid) || visible.contains(&xid) {
                ledger.acknowledge(xid);
            } else {
                ledger.abort(xid);
            }
        }
    }
}

#[derive(Debug, Default)]
struct Tally {
    crashes: usize,
    acked: usize,
    findings: usize,
    unopenable: usize,
}

/// Crash a random workload `rounds` times under `faults`, reopening and checking after each.
fn crash_loop(
    seed: u64,
    faults: Faults,
    layout: Layout,
    damage_allowed: bool,
    rounds: usize,
) -> Tally {
    silence_fatal_panics();
    let mut sim = Sim::new(
        seed,
        Config {
            faults: quiet(),
            atomic_unit: if layout.atomic { PAGE as u64 } else { 512 },
            ..Config::default()
        },
    );
    let client = Client::new();
    setup(&mut sim, &client, layout);
    let mut tally = Tally::default();
    for round in 0..rounds {
        sim.set_faults(faults.clone());
        let at = sim.now()
            + Duration::from_micros(500 + (seed * 7919 + round as u64 * 104_729) % 60_000);
        sim.crash_at(at);
        let c = client.clone();
        let body = |rt: SimRuntime| {
            if layout.threads == 0 {
                workload(rt, &c, layout, 400);
            } else {
                concurrent(rt, &c, layout, 150);
            }
        };
        if run_process(&mut sim, body) == Outcome::Crashed {
            tally.crashes += 1;
        }
        sim.set_faults(quiet());
        let pages = client.world.lock().unwrap().pages.clone();
        let mut report = Report::default();
        let outcome = sim.run(|rt| report = inspect(rt, &pages, layout));
        assert_eq!(Outcome::Finished, outcome);
        let world = client.world.lock().unwrap();
        let problems = judge(
            &report,
            &client.ledger,
            &world,
            &sim.trace(),
            damage_allowed,
        );
        assert!(
            problems.is_empty(),
            "seed {seed} round {round}: {problems:#?}\nfindings {:?}",
            report.findings
        );
        tally.findings += report.findings.len();
        resolve(&report, &client.ledger, &world);
        if report.open.is_some() {
            tally.unopenable += 1;
            break;
        }
    }
    tally.acked = client
        .world
        .lock()
        .unwrap()
        .writes
        .keys()
        .filter(|&&x| client.ledger.fate(x) == Some(Fate::Acknowledged))
        .count();
    tally
}

fn seeds(
    range: std::ops::Range<u64>,
    faults: Faults,
    layout: Layout,
    damage_allowed: bool,
) -> Tally {
    let mut total = Tally::default();
    for seed in range {
        let t = crash_loop(seed, faults.clone(), layout, damage_allowed, 4);
        total.crashes += t.crashes;
        total.acked += t.acked;
        total.findings += t.findings;
        total.unopenable += t.unopenable;
    }
    eprintln!("{total:?}");
    total
}

#[test]
fn crashes_with_torn_writes_lose_nothing_and_find_nothing() {
    let t = seeds(
        0..12,
        Faults {
            torn_p: 0.5,
            ..quiet()
        },
        PLAIN,
        false,
    );
    assert!(t.crashes > 20, "{t:?}");
    assert!(t.acked > 200, "{t:?}");
}

#[test]
fn torn_log_blocks_lose_no_acknowledged_commit() {
    let t = seeds(
        600..612,
        Faults {
            torn_p: 1.0,
            ..quiet()
        },
        Layout {
            atomic: false,
            log_block: 2048,
            threads: 0,
            frames: 5,
        },
        false,
    );
    assert!(t.crashes > 20, "{t:?}");
    assert!(t.acked > 200, "{t:?}");
}

#[test]
fn atomic_page_writes_need_no_images() {
    let t = seeds(
        100..106,
        Faults {
            torn_p: 0.5,
            ..quiet()
        },
        Layout {
            atomic: true,
            log_block: 512,
            threads: 0,
            frames: 5,
        },
        false,
    );
    assert!(t.crashes > 10, "{t:?}");
}

#[test]
fn bit_rot_and_latent_errors_are_reported_never_returned() {
    let t = seeds(
        200..212,
        Faults {
            torn_p: 0.3,
            corrupt_p: 0.004,
            latent_p: 0.002,
            ..quiet()
        },
        PLAIN,
        true,
    );
    assert!(t.findings > 0, "no seed corrupted anything: {t:?}");
}

#[test]
fn misdirected_and_lost_writes_are_reported_never_returned() {
    let t = seeds(
        300..312,
        Faults {
            torn_p: 0.3,
            misdirect_p: 0.004,
            lost_write_p: 0.004,
            ..quiet()
        },
        PLAIN,
        true,
    );
    assert!(t.findings > 0, "no seed lost a write: {t:?}");
}

#[test]
fn failed_writes_and_forces_crash_and_recover() {
    let t = seeds(
        400..412,
        Faults {
            torn_p: 0.3,
            fail_p: 0.004,
            ..quiet()
        },
        PLAIN,
        false,
    );
    assert!(t.crashes > 20, "{t:?}");
}

#[test]
fn every_fault_at_once() {
    seeds(
        500..512,
        Faults {
            torn_p: 0.3,
            corrupt_p: 0.002,
            latent_p: 0.001,
            misdirect_p: 0.002,
            lost_write_p: 0.002,
            fail_p: 0.002,
            ..quiet()
        },
        PLAIN,
        true,
    );
}

#[test]
fn a_seed_replays_exactly() {
    let run = |seed| {
        let mut sim = Sim::new(seed, Config::default());
        let client = Client::new();
        setup(&mut sim, &client, PLAIN);
        sim.crash_at(sim.now() + Duration::from_millis(20));
        let c = client.clone();
        run_process(&mut sim, |rt| workload(rt, &c, PLAIN, 200));
        (sim.trace(), sim.files())
    };
    assert_eq!(run(9), run(9));
}

fn with_pager<T>(sim: &mut Sim, body: impl FnOnce(&mut Pager<SimRuntime>) -> T) -> T {
    let mut out = None;
    let outcome = sim.run(|rt| {
        let (mut p, _) = Pager::open(rt, &options(PLAIN)).expect("open");
        out = Some(body(&mut p));
    });
    assert_eq!(Outcome::Finished, outcome);
    out.unwrap()
}

/// Returns a copy of `sim`'s device after a clean reopen, with one bit of `path` flipped.
fn flip(sim: &mut Sim, path: &str, at: usize) -> Sim {
    with_pager(sim, |_| ());
    let mut files = sim.files();
    files.get_mut(path).unwrap()[at] ^= 0x20;
    Sim::with_files(1, Config::default(), files)
}

#[test]
fn a_rotted_page_is_refused_and_reported() {
    let mut sim = Sim::new(1, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let pid = client.world.lock().unwrap().pages[3];
    let mut sim = flip(&mut sim, NAME, pid.0 as usize * PAGE + 1000);
    let (read, findings) = with_pager(&mut sim, |p| {
        let read = p.read(pid).map(|_| ()).map_err(|e| e.damage);
        (read, p.verify().findings)
    });
    assert_eq!(Err(Damage::Checksum), read);
    assert!(findings.contains(&Finding::Page {
        pid,
        damage: Damage::Checksum
    }));
}

#[test]
fn a_damaged_bitmap_fails_allocation() {
    let mut sim = Sim::new(2, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let mut sim = flip(&mut sim, NAME, 2 * PAGE + 500);
    let got = with_pager(&mut sim, |p| p.allocate(1, PageKind::HeapSlotted, 7));
    assert_eq!(
        Pid(2),
        got.expect_err("allocation used a damaged bitmap").pid
    );
}

#[test]
fn allocation_skips_a_damaged_free_page() {
    let mut sim = Sim::new(3, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let next = Pid(3 + PAGES as u64);
    let mut files = sim.files();
    let data = files.get_mut(NAME).unwrap();
    data.resize((next.0 as usize + 1) * PAGE, 0);
    data[next.0 as usize * PAGE + 77] = 1;
    let mut sim = Sim::with_files(3, Config::default(), files);
    let got = with_pager(&mut sim, |p| p.allocate(1, PageKind::HeapSlotted, 7));
    assert_eq!(Ok(Pid(next.0 + 1)), got);
}

#[test]
fn allocations_survive_crashes_and_never_repeat() {
    for seed in 0..8 {
        let mut sim = Sim::new(
            seed,
            Config {
                faults: Faults {
                    torn_p: 0.5,
                    fail_p: 0.01,
                    ..quiet()
                },
                ..Config::default()
            },
        );
        silence_fatal_panics();
        let client = Client::new();
        sim.set_faults(quiet());
        setup(&mut sim, &client, PLAIN);
        let durable: Arc<Mutex<BTreeSet<Pid>>> = Arc::default();
        for round in 0..4u64 {
            sim.set_faults(Faults {
                torn_p: 0.5,
                fail_p: 0.01,
                ..quiet()
            });
            sim.crash_at(sim.now() + Duration::from_micros(300 + (seed * 31 + round * 977) % 9000));
            let d = durable.clone();
            run_process(&mut sim, |rt| {
                let Ok((p, _)) = Pager::open(rt, &options(PLAIN)) else {
                    return;
                };
                loop {
                    let pid = p.allocate(1, PageKind::HeapSlotted, 9).expect("allocate");
                    p.flush_log();
                    assert!(d.lock().unwrap().insert(pid), "{pid:?} allocated twice");
                    if pid.0.is_multiple_of(5) {
                        d.lock().unwrap().remove(&pid);
                        p.free(pid).expect("free");
                        p.flush_log();
                    }
                }
            });
        }
        sim.set_faults(quiet());
        let held = durable.lock().unwrap().clone();
        with_pager(&mut sim, |p| {
            let fresh = p.allocate(1, PageKind::HeapSlotted, 9).expect("allocate");
            assert!(
                !held.contains(&fresh),
                "seed {seed}: {fresh:?} handed out twice"
            );
            assert!(p.verify().findings.is_empty());
        });
    }
}

#[test]
fn crash_replay_at_every_flush_boundary() {
    let mut sim = Sim::new(
        11,
        Config {
            faults: Faults {
                delay_completions: true,
                ..Faults::default()
            },
            ..Config::default()
        },
    );
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    sim.record();
    let c = client.clone();
    assert_eq!(Outcome::Finished, sim.run(|rt| workload(rt, &c, PLAIN, 60)));
    let recording = sim.recording();
    let world = client.world.lock().unwrap();
    let mut images = 0;
    for at in recording.flushes() {
        for files in recording.images(at, 8, at as u64) {
            images += 1;
            let ledger = Ledger::new();
            for (&xid, &stamp) in &world.submitted {
                if stamp <= at {
                    ledger.submit(xid);
                    match world.acked.get(&xid) {
                        Some(&a) if a <= at => ledger.acknowledge(xid),
                        _ if client.ledger.fate(xid) == Some(Fate::Aborted) => ledger.abort(xid),
                        _ => {}
                    }
                }
            }
            let mut replay = Sim::with_files(at as u64, Config::default(), files);
            let mut report = Report::default();
            assert_eq!(
                Outcome::Finished,
                replay.run(|rt| report = inspect(rt, &world.pages, PLAIN))
            );
            let problems = judge(&report, &ledger, &world, "", false);
            assert!(problems.is_empty(), "flush boundary {at}: {problems:#?}");
        }
    }
    assert!(
        recording.flushes().len() > 20,
        "{} flushes",
        recording.flushes().len()
    );
    assert!(images > 100, "{images} images");
}

const THREADED: Layout = Layout {
    atomic: false,
    log_block: 512,
    threads: 4,
    frames: 14,
};

#[test]
fn threads_sharing_a_pager_lose_nothing_across_crashes() {
    let t = seeds(
        700..710,
        Faults {
            torn_p: 0.5,
            ..quiet()
        },
        THREADED,
        false,
    );
    assert!(t.crashes > 10, "{t:?}");
    assert!(t.acked > 100, "{t:?}");
}

#[test]
fn threads_sharing_a_pager_under_every_fault() {
    seeds(
        800..808,
        Faults {
            torn_p: 0.3,
            corrupt_p: 0.002,
            latent_p: 0.001,
            misdirect_p: 0.002,
            lost_write_p: 0.002,
            fail_p: 0.002,
            ..quiet()
        },
        THREADED,
        true,
    );
}

#[test]
fn threads_finish_and_every_acknowledged_write_stays() {
    for seed in 900..904 {
        let mut sim = Sim::new(seed, Config::default());
        let client = Client::new();
        setup(&mut sim, &client, THREADED);
        let c = client.clone();
        assert_eq!(
            Outcome::Finished,
            sim.run(|rt| concurrent(rt, &c, THREADED, 120))
        );
        let pages = client.world.lock().unwrap().pages.clone();
        let mut report = Report::default();
        assert_eq!(
            Outcome::Finished,
            sim.run(|rt| report = inspect(rt, &pages, THREADED))
        );
        let world = client.world.lock().unwrap();
        let problems = judge(&report, &client.ledger, &world, "", false);
        assert!(problems.is_empty(), "seed {seed}: {problems:#?}");
        let acked = world
            .writes
            .keys()
            .filter(|&&x| client.ledger.fate(x) == Some(Fate::Acknowledged))
            .count();
        assert_eq!(
            world.writes.len(),
            acked,
            "every transaction ends acknowledged"
        );
    }
}

fn reacquire(second: impl FnOnce(&Pager<SimRuntime>, Pid)) {
    let mut sim = Sim::new(21, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let pid = client.world.lock().unwrap().pages[0];
    with_pager(&mut sim, |p| {
        let held = p.read(pid).unwrap();
        second(p, pid);
        drop(held);
    });
}

#[test]
#[should_panic(expected = "is already latched by this thread")]
fn changing_a_page_while_holding_its_shared_latch_panics() {
    reacquire(|p, pid| {
        p.update(&[pid], |_| {}).unwrap();
    });
}

#[test]
#[should_panic(expected = "is already latched by this thread")]
fn reading_a_page_while_holding_its_exclusive_latch_panics() {
    let mut sim = Sim::new(22, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let pid = client.world.lock().unwrap().pages[0];
    with_pager(&mut sim, |p| {
        let held = p.exclusive(pid).unwrap();
        drop(p.read(pid));
        drop(held);
    });
}

#[test]
#[should_panic(expected = "is already latched by this thread")]
fn an_optimistic_read_of_a_page_held_exclusive_panics() {
    let mut sim = Sim::new(23, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let pid = client.world.lock().unwrap().pages[0];
    with_pager(&mut sim, |p| {
        let held = p.exclusive(pid).unwrap();
        let _ = p.optimistic(pid);
        drop(held);
    });
}

#[test]
fn the_panic_names_the_page() {
    let mut sim = Sim::new(24, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let pid = client.world.lock().unwrap().pages[0];
    let caught = panic::catch_unwind(AssertUnwindSafe(|| {
        with_pager(&mut sim, |p| {
            let held = p.read(pid).unwrap();
            drop(p.exclusive(pid));
            drop(held);
        })
    }));
    let payload = caught.expect_err("asking again for a held latch returned");
    let msg = payload
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        format!("page {} is already latched by this thread", pid.0),
        msg
    );
}

#[test]
fn releasing_a_latch_lets_the_thread_take_it_again() {
    reacquire(|_, _| {});
    let mut sim = Sim::new(25, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let pid = client.world.lock().unwrap().pages[0];
    with_pager(&mut sim, |p| {
        drop(p.read(pid).unwrap());
        p.update(&[pid], |_| {}).unwrap();
        drop(p.exclusive(pid).unwrap());
        assert!(p.optimistic(pid).unwrap().validate());
    });
}

/// Returns the pages the allocation bitmap of group 0 marks allocated.
fn allocated(p: &Pager<SimRuntime>) -> BTreeSet<Pid> {
    let map = p.read(Pid(2)).unwrap().to_vec();
    (0..(PAGE - HEADER_LEN) * 8)
        .filter(|&i| map[HEADER_LEN + i / 8] >> (i % 8) & 1 == 1)
        .map(|i| Pid(i as u64))
        .filter(|&pid| pid != Pid(2))
        .collect()
}

/// Returns the pages the link page `index` names, one per nonzero slot.
fn linked(p: &Pager<SimRuntime>, index: Pid) -> BTreeSet<Pid> {
    let page = p.read(index).unwrap().to_vec();
    (0..SLOTS)
        .map(|s| {
            u64::from_le_bytes(
                page[HEADER_LEN + s * 8..HEADER_LEN + s * 8 + 8]
                    .try_into()
                    .unwrap(),
            )
        })
        .filter(|&v| v != 0)
        .map(Pid)
        .collect()
}

#[test]
fn a_crash_never_separates_an_allocation_from_its_link() {
    silence_fatal_panics();
    let mut leaks_possible_before = 0;
    for seed in 0..10 {
        let mut sim = Sim::new(seed, Config::default());
        let client = Client::new();
        setup(&mut sim, &client, PLAIN);
        let index = client.world.lock().unwrap().pages[0];
        let before = with_pager(&mut sim, |p| allocated(p));
        for round in 0..4u64 {
            sim.set_faults(Faults {
                torn_p: 0.5,
                ..quiet()
            });
            sim.crash_at(
                sim.now()
                    + Duration::from_micros(20_000 + (seed * 7919 + round * 104_729) % 80_000),
            );
            run_process(&mut sim, |rt| {
                let Ok((p, _)) = Pager::open(rt, &options(PLAIN)) else {
                    return;
                };
                let used = linked(&p, index).len();
                for slot in used..SLOTS {
                    let mut new = p.reserve();
                    let pid = new.page(1, PageKind::HeapSlotted, 9).unwrap();
                    p.update_with(new, &[index, pid], |pages| {
                        let at = HEADER_LEN + slot * 8;
                        pages[0][at..at + 8].copy_from_slice(&pid.0.to_le_bytes());
                        pages[1][HEADER_LEN] = 0xab;
                    })
                    .unwrap();
                    if slot % 7 == 0 {
                        p.flush_log();
                    }
                    leaks_possible_before += 1;
                }
            });
            sim.set_faults(quiet());
            with_pager(&mut sim, |p| {
                let fresh: BTreeSet<Pid> = allocated(p).difference(&before).copied().collect();
                let links = linked(p, index);
                assert_eq!(
                    fresh, links,
                    "seed {seed} round {round}: allocations and links differ"
                );
                for &pid in &links {
                    let page = p.read(pid).unwrap();
                    assert_eq!(PageKind::HeapSlotted as u16, adtb::storage::kind(&page));
                    assert_eq!(9, adtb::storage::owner(&page));
                    assert_eq!(0xab, page[HEADER_LEN]);
                    assert!(page[HEADER_LEN + 1..].iter().all(|&b| b == 0));
                }
            });
        }
    }
    assert!(
        leaks_possible_before > 50,
        "{leaks_possible_before} allocations"
    );
}

#[test]
fn a_dropped_reservation_gives_its_pages_back() {
    let mut sim = Sim::new(31, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    with_pager(&mut sim, |p| {
        let mut a = p.reserve();
        let x = a.page(1, PageKind::HeapSlotted, 1).unwrap();
        let mut b = p.reserve();
        let y = b.page(1, PageKind::HeapSlotted, 1).unwrap();
        assert_ne!(x, y, "two reservations hold one page");
        assert!(
            !allocated(p).contains(&x),
            "a reservation marks nothing allocated"
        );
        drop(a);
        assert_eq!(Ok(x), p.allocate(1, PageKind::HeapSlotted, 1));
        drop(b);
        assert_eq!(Ok(y), p.allocate(1, PageKind::HeapSlotted, 1));
    });
}

#[test]
fn guards_allocate_and_link_in_one_batch() {
    let mut sim = Sim::new(32, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let index = client.world.lock().unwrap().pages[1];
    let (pid, lsns) = with_pager(&mut sim, |p| {
        let mut new = p.reserve();
        let pid = new.page(3, PageKind::IndexLeaf, 4).unwrap();
        assert_eq!(
            PageKind::IndexLeaf as u16,
            adtb::storage::kind(&new.image(pid))
        );
        let mut parent = p.exclusive(index).unwrap();
        let mut child = p.exclusive(pid).unwrap();
        let before = p.durable();
        p.update_all_with(new, &mut [&mut parent, &mut child], |pages| {
            pages[0][HEADER_LEN..HEADER_LEN + 8].copy_from_slice(&pid.0.to_le_bytes());
            pages[1][HEADER_LEN] = 1;
        })
        .unwrap();
        drop((parent, child));
        assert_eq!(before, p.durable(), "nothing is flushed until asked");
        p.flush_log();
        (pid, allocated(p))
    });
    assert!(lsns.contains(&pid));
    with_pager(&mut sim, |p| {
        assert_eq!(BTreeSet::from([pid]), linked(p, index));
        assert_eq!(3, adtb::storage::format(&p.read(pid).unwrap()));
    });
}

#[test]
#[should_panic(expected = "is not among the pages changed")]
fn an_update_must_change_every_page_it_allocates() {
    let mut sim = Sim::new(33, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let index = client.world.lock().unwrap().pages[1];
    with_pager(&mut sim, |p| {
        let mut new = p.reserve();
        new.page(1, PageKind::HeapSlotted, 1).unwrap();
        let _ = p.update_with(new, &[index], |_| {});
    });
}

#[test]
fn a_damaged_bitmap_fails_a_reservation() {
    let mut sim = Sim::new(34, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let mut sim = flip(&mut sim, NAME, 2 * PAGE + 500);
    let got = with_pager(&mut sim, |p| p.reserve().page(1, PageKind::HeapSlotted, 7));
    assert_eq!(
        Pid(2),
        got.expect_err("reserved through a damaged bitmap").pid
    );
}

#[test]
fn a_lost_write_is_caught_when_the_page_is_read_back() {
    let mut sim = Sim::new(41, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let pages = client.world.lock().unwrap().pages.clone();
    let victim = pages[0];
    let outcome = sim.run(|rt| {
        let (p, _) = Pager::open(rt.clone(), &options(PLAIN)).unwrap();
        for &pid in &pages {
            p.update(&[pid], |v| v[0][HEADER_LEN] ^= 1).unwrap();
        }
        p.flush_pages(usize::MAX);
        p.update(&[victim], |v| v[0][HEADER_LEN + 1] = 7).unwrap();
        p.flush_log();
        rt.set_faults(Faults {
            lost_write_p: 1.0,
            ..quiet()
        });
        p.flush_pages(usize::MAX);
        rt.set_faults(quiet());
        for _ in 0..3 {
            for &pid in &pages[1..] {
                drop(p.read(pid).unwrap());
            }
        }
        let back = p.read(victim).map(|_| ()).map_err(|e| e.damage);
        assert!(
            matches!(back, Err(Damage::Stale { .. })),
            "a lost write read back as {back:?}"
        );
    });
    assert_eq!(Outcome::Finished, outcome);
}

#[test]
fn a_crash_never_separates_freeing_from_unlinking() {
    silence_fatal_panics();
    let mut freed = 0;
    for seed in 0..10 {
        let mut sim = Sim::new(seed + 50, Config::default());
        let client = Client::new();
        setup(&mut sim, &client, PLAIN);
        let index = client.world.lock().unwrap().pages[0];
        let before = with_pager(&mut sim, |p| allocated(p));
        for round in 0..4u64 {
            sim.set_faults(Faults {
                torn_p: 0.5,
                ..quiet()
            });
            sim.crash_at(
                sim.now()
                    + Duration::from_micros(20_000 + (seed * 7919 + round * 104_729) % 80_000),
            );
            run_process(&mut sim, |rt| {
                let Ok((p, _)) = Pager::open(rt.clone(), &options(PLAIN)) else {
                    return;
                };
                for _ in 0..400 {
                    let page = p.read(index).unwrap().to_vec();
                    let at = |s: usize| HEADER_LEN + s * 8;
                    let pid_at =
                        |s: usize| u64::from_le_bytes(page[at(s)..at(s) + 8].try_into().unwrap());
                    let s = (rt.next_u64() % 40) as usize;
                    let pid = pid_at(s);
                    if pid == 0 {
                        let mut new = p.reserve();
                        let fresh = new.page(1, PageKind::HeapSlotted, 9).unwrap();
                        p.update_with(new, &[index, fresh], |v| {
                            v[0][at(s)..at(s) + 8].copy_from_slice(&fresh.0.to_le_bytes());
                        })
                        .unwrap();
                    } else {
                        let mut new = p.reserve();
                        new.free(Pid(pid));
                        p.update_with(new, &[index], |v| v[0][at(s)..at(s) + 8].fill(0))
                            .unwrap();
                        freed += 1;
                    }
                    if rt.next_u64().is_multiple_of(5) {
                        p.flush_log();
                    }
                }
            });
            sim.set_faults(quiet());
            with_pager(&mut sim, |p| {
                let fresh: BTreeSet<Pid> = allocated(p).difference(&before).copied().collect();
                assert_eq!(
                    fresh,
                    linked(p, index),
                    "seed {seed} round {round}: allocations and links differ"
                );
            });
        }
    }
    assert!(freed > 50, "{freed} frees");
}

#[test]
#[should_panic(expected = "is not allocated")]
fn freeing_a_free_page_panics() {
    let mut sim = Sim::new(35, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    with_pager(&mut sim, |p| {
        let pid = p.allocate(1, PageKind::HeapSlotted, 1).unwrap();
        p.free(pid).unwrap();
        let mut new = p.reserve();
        new.free(pid);
        let _ = p.update_with(new, &[], |_| {});
    });
}

#[test]
fn a_reservation_dropped_unused_frees_nothing() {
    let mut sim = Sim::new(36, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let pid = client.world.lock().unwrap().pages[2];
    with_pager(&mut sim, |p| {
        let mut new = p.reserve();
        new.free(pid);
        drop(new);
        assert!(allocated(p).contains(&pid));
    });
}

#[test]
#[should_panic(expected = "every frame of the pool")]
fn latching_every_frame_from_one_thread_panics() {
    let mut sim = Sim::new(37, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let pages = client.world.lock().unwrap().pages.clone();
    with_pager(&mut sim, |p| {
        let held: Vec<_> = pages[..PLAIN.frames]
            .iter()
            .map(|&pid| p.exclusive(pid).unwrap())
            .collect();
        drop(p.read(pages[PLAIN.frames]));
        drop(held);
    });
}
