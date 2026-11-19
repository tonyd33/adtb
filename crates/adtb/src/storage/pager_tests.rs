use crate::testing::Probe;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::runtime::{Entropy, FatalIo, IoError, OpKind, Runtime};
use crate::storage::{
    Damage, Edit, Finding, HEADER_LEN, LATCHES_PER_THREAD, LogFinding, LogProblem, Lsn, OpenError,
    Options, PageError, PageKind, Pager, Pid,
};
use adtb_sim::{
    Action, Config, Fate, FindingKind, Ledger, Match, Outcome, Plan, Recovered, Sim, SimRuntime,
    Violation, check,
};
use adtb_sys::os::{Backend, OsRuntime};
use adtb_sys::runtime::Faults;
use std::cell::Cell;

use adtb_testkit::Rng;
use adtb_testkit::crash;
use adtb_testkit::damage::DataFile;
use adtb_testkit::scratch::Scratch;
use adtb_testkit::seeds;

use Device::{Pool, Simulated, Uring};
use Fault::{Every, Failed, Misdirected, Rot, Torn, TornAll};
use Shape::{Atomic, Everything, Plain, Threaded, WideLog};
use adtb_testkit::profile::every_case;
use crash::{Reopen, Reopened, Rounds, crash_loop, run_process, silence_fatal_panics};
use seeds::{assert_reached, share, sweep};

const PAGE: usize = 4096;
const SLOT: usize = 16;
const SLOTS: usize = (PAGE - HEADER_LEN) / SLOT;
const PAGES: usize = 24;
const NAME: &str = "app.adt";
const DATA: DataFile = DataFile::new(NAME, PAGE);
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

const THREADED: Layout = Layout {
    atomic: false,
    log_block: 512,
    threads: 4,
    frames: 20,
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
        threads: layout.threads.max(1),
        max_pages: 4096,
        atomic_page_writes: atomic,
        ..Options::new(NAME)
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

/// The draws a workload takes and the count of device events recorded so far.
#[derive(Clone)]
struct Host {
    draw: Arc<dyn Fn() -> u64 + Send + Sync>,
    stamp: Arc<dyn Fn() -> usize + Send + Sync>,
}

impl Host {
    /// Returns the simulator's draws and its recorded events.
    fn sim(rt: &SimRuntime) -> Host {
        let (draws, events) = (rt.clone(), rt.clone());
        Host {
            draw: Arc::new(move || draws.next_u64()),
            stamp: Arc::new(move || events.recorded()),
        }
    }

    /// Returns draws that depend only on `seed`, with no events recorded.
    fn seeded(seed: u64) -> Host {
        let rng = Mutex::new(Rng::new(seed));
        Host {
            draw: Arc::new(move || rng.lock().unwrap().next_u64()),
            stamp: Arc::new(|| 0),
        }
    }

    fn draw(&self) -> u64 {
        (self.draw)()
    }

    fn stamp(&self) -> usize {
        (self.stamp)()
    }
}

/// Create the database with `PAGES` pages on `rt`, every allocation durable.
fn create<R: Runtime>(rt: R, client: &Client, layout: Layout) {
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
    client.world.lock().unwrap().pages = pages;
}

fn setup(sim: &mut Sim, client: &Client, layout: Layout) {
    let outcome = sim.run(|rt| create(rt, client, layout));
    assert_eq!(Outcome::Finished, outcome);
}

/// Open the database and run random transactions, flushes and checkpoints for `steps` steps.
fn workload<R: Runtime>(rt: R, host: &Host, client: &Client, layout: Layout, steps: usize) {
    let Ok((mut pager, _)) = Pager::open(rt, &options(layout)) else {
        return;
    };
    let pages = client.world.lock().unwrap().pages.clone();
    let mut unacked: Vec<u64> = Vec::new();
    let ack = |pager: &mut Pager<R>, unacked: &mut Vec<u64>| {
        pager.flush_log();
        let stamp = host.stamp();
        let mut w = client.world.lock().unwrap();
        for xid in unacked.drain(..) {
            client.ledger.acknowledge(xid);
            w.acked.insert(xid, stamp);
        }
    };
    for _ in 0..steps {
        match host.draw() % 20 {
            0..=13 => {
                let n = 1 + (host.draw() % 3) as usize;
                let mut chosen = BTreeSet::new();
                while chosen.len() < n {
                    chosen.insert(pages[(host.draw() % pages.len() as u64) as usize]);
                }
                let xid = {
                    let mut w = client.world.lock().unwrap();
                    w.next_xid += 1;
                    w.next_xid
                };
                let mut writes = Vec::new();
                let mut changes = Vec::new();
                for pid in chosen {
                    let start = (host.draw() % SLOTS as u64) as usize;
                    let len = 1 + (host.draw() % 6) as usize;
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
                    w.submitted.insert(xid, host.stamp());
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
                if host.draw().is_multiple_of(2) {
                    ack(&mut pager, &mut unacked);
                }
            }
            14..=16 => {
                let n = 1 + (host.draw() % 4) as usize;
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
fn worker<R: Runtime>(pager: &Pager<R>, rt: &R, host: &Host, client: &Client, steps: usize) {
    let pages = client.world.lock().unwrap().pages.clone();
    let pick = || pages[(host.draw() % pages.len() as u64) as usize];
    let mut unacked: Vec<u64> = Vec::new();
    let ack = |unacked: &mut Vec<u64>| {
        pager.flush_log();
        let stamp = host.stamp();
        let mut w = client.world.lock().unwrap();
        for xid in unacked.drain(..) {
            client.ledger.acknowledge(xid);
            w.acked.insert(xid, stamp);
        }
    };
    for _ in 0..steps {
        match host.draw() % 12 {
            0..=6 => {
                let n = 1 + (host.draw() % 3) as usize;
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
                    let start = (host.draw() % SLOTS as u64) as usize;
                    let len = 1 + (host.draw() % 6) as usize;
                    writes.push((pid, (start..(start + len).min(SLOTS)).collect::<Vec<_>>()));
                }
                let xid = {
                    let mut w = client.world.lock().unwrap();
                    w.next_xid += 1;
                    let xid = w.next_xid;
                    w.writes.insert(xid, writes.clone());
                    w.submitted.insert(xid, host.stamp());
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
                if host.draw().is_multiple_of(3) {
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
            _ if host.draw().is_multiple_of(4) => pager.checkpoint(),
            _ => ack(&mut unacked),
        }
    }
    ack(&mut unacked);
}

/// Open the database and run `layout.threads` workers over it for `steps` steps each.
fn concurrent<R: Runtime>(rt: R, host: &Host, client: &Client, layout: Layout, steps: usize) {
    let Ok((pager, _)) = Pager::open(rt.clone(), &options(layout)) else {
        return;
    };
    let workers: Vec<_> = (0..layout.threads)
        .map(|i| {
            let (pager, rt2, host, client) =
                (pager.clone(), rt.clone(), host.clone(), client.clone());
            rt.spawn(&format!("worker {i}"), move || {
                worker(&pager, &rt2, &host, &client, steps);
            })
        })
        .collect();
    for w in workers {
        rt.join(w);
    }
}

/// Run the workload `layout` asks for: `steps` steps, shared among its threads when it has any.
fn drive<R: Runtime>(rt: R, host: &Host, client: &Client, layout: Layout, steps: usize) {
    if layout.threads == 0 {
        workload(rt, host, client, layout, steps);
    } else {
        concurrent(rt, host, client, layout, steps * 3 / 8);
    }
}

/// What a reopened database shows the checks.
#[derive(Debug, Default)]
struct Report {
    open: Option<OpenError>,
    findings: Vec<Finding>,
    commits: BTreeSet<u64>,
    pages: BTreeMap<Pid, Option<Vec<u8>>>,
    /// Whether the faults may damage the file.
    damage_allowed: bool,
    /// Whether the log was found damaged or lost a write.
    log_trouble: bool,
    /// Each way the pages disagree with what the client wrote.
    problems: Vec<String>,
    /// Every transaction a page shows.
    visible: BTreeSet<u64>,
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
            .chain(self.problems.iter().map(|p| adtb_sim::Finding {
                kind: FindingKind::Logic,
                at: p.clone(),
            }))
            .collect()
    }
}

impl Reopened for Report {
    fn allows(&self, v: &Violation) -> bool {
        match v {
            Violation::Lost(_) => self.log_trouble || self.open.is_some(),
            Violation::Found(f) => self.damage_allowed && f.kind == FindingKind::Structure,
            Violation::Phantom(_) => false,
        }
    }

    /// Acknowledge each commit in doubt that the log holds or a page shows, and abort the rest.
    fn settle(&self, ledger: &Ledger) {
        ledger.settle(&self.commits.union(&self.visible).copied().collect());
    }

    fn last(&self) -> bool {
        self.open.is_some()
    }
}

fn inspect<R: Runtime>(rt: R, pages: &[Pid], layout: Layout) -> Report {
    let (pager, opening) = match Pager::open(rt, &options(layout)) {
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
    let recovery = opening.recovery().cloned().expect("the open recovered");
    findings.extend(recovery.log.into_iter().map(Finding::Log));
    findings.sort();
    findings.dedup();
    let mut report = Report {
        open: None,
        findings,
        commits: v.commits.into_iter().collect(),
        ..Report::default()
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

/// Record in `report` each way its pages disagree with the client's records, given the fates of
/// `ledger`, and every transaction they show. `lost_log_write` tells whether the faults lost a write
/// to the log.
fn judge(report: &mut Report, ledger: &Ledger, world: &World, lost_log_write: bool) {
    if let Some(e) = report.open {
        let damage = matches!(
            e,
            OpenError::NoHeader | OpenError::Log(_) | OpenError::Page(_)
        );
        if !(damage && report.damage_allowed) {
            report.problems.push(format!("open failed: {e:?}"));
        }
        return;
    }
    report.log_trouble =
        report.findings.iter().any(|f| matches!(f, Finding::Log(_))) || lost_log_write;
    let mut problems = Vec::new();
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
        report
            .visible
            .extend(shown.iter().copied().filter(|&x| x != 0));
        for (s, &x) in shown.iter().enumerate() {
            if x != 0 && !report.log_trouble && !report.commits.contains(&x) {
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
            if let Some((w, _)) = newer_acked
                && !report.log_trouble
            {
                problems.push(format!(
                    "{pid:?} slot {s} shows {x}, losing acknowledged {w}"
                ));
            }
        }
    }
    report.problems = problems;
}

/// Returns each violation of `ledger` and each page problem `report` shows, with no damage
/// allowed and no log write lost.
fn judged(mut report: Report, ledger: &Ledger, world: &World) -> Vec<String> {
    judge(&mut report, ledger, world, false);
    check(ledger, &report)
        .into_iter()
        .filter(|v| !report.allows(v))
        .map(|v| format!("{v:?}"))
        .collect()
}

/// The fewest crashes under every fault that may leave one file refusing to open: the measured
/// refused-open rate is about 7% of crashes, and up to 25% over 8 seeds, so the bound is 33%.
const CRASHES_PER_REFUSAL: usize = 3;

/// The least a row of the crash table reaches over all of its seeds, its kinds of finding only
/// under the nextest profile `deep`.
#[derive(Clone, Copy, Debug)]
struct Floors {
    /// Strikes of the row's fault.
    struck: usize,
    crashes: usize,
    acked: usize,
    reopened: usize,
    pages: Rate,
    logs: Rate,
}

/// At least `found` findings of one kind per `per` seeds run.
#[derive(Clone, Copy, Debug)]
struct Rate {
    found: usize,
    per: usize,
}

const NO_KIND: Rate = Rate { found: 0, per: 1 };

/// Returns floors of one strike and of `crashes`, `acked` and `reopened`, with no kind of
/// finding.
const fn floors(crashes: usize, acked: usize, reopened: usize) -> Floors {
    Floors {
        struck: 1,
        crashes,
        acked,
        reopened,
        pages: NO_KIND,
        logs: NO_KIND,
    }
}

/// Page findings under misdirected and lost writes: under half the measured 104 over 64 seeds.
const MISDIRECTED_PAGES: Rate = Rate { found: 1, per: 2 };
/// Log findings under misdirected and lost writes: one, as 6 of 64 measured seeds reach any and
/// 96 seeds then all miss with chance under 1e-4.
const MISDIRECTED_LOGS: Rate = Rate { found: 1, per: 96 };
/// Page findings under every fault: under half the measured 85 over 64 seeds.
const EVERY_FAULT_PAGES: Rate = Rate { found: 1, per: 2 };
/// Reopens of 8 seeds of threads under every fault: half the measured mean of 3.33 per seed over
/// 256 seeds.
const THREADED_REOPENS: usize = 13;
/// Page findings of threads under every fault: under half the measured 305 over 256 seeds.
const THREADED_PAGES: Rate = Rate { found: 1, per: 2 };
/// Log findings of threads under every fault: one, as 25 of 256 measured seeds reach any and 64
/// seeds then all miss with chance under 0.2%.
const THREADED_LOGS: Rate = Rate { found: 1, per: 64 };

/// The layouts of the crash table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Plain,
    WideLog,
    Atomic,
    Threaded,
    /// Four threads, atomic page writes and wide log blocks.
    Everything,
}

impl Shape {
    const ALL: [Shape; 5] = [Plain, WideLog, Atomic, Threaded, Everything];

    fn layout(self) -> Layout {
        match self {
            Plain => PLAIN,
            WideLog => Layout {
                log_block: 2048,
                ..PLAIN
            },
            Atomic => Layout {
                atomic: true,
                ..PLAIN
            },
            Threaded => THREADED,
            Everything => Layout {
                atomic: true,
                log_block: 2048,
                ..THREADED
            },
        }
    }
}

/// The faults of the crash table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    Torn,
    TornAll,
    Rot,
    Misdirected,
    Failed,
    Every,
}

impl Fault {
    const ALL: [Fault; 6] = [Torn, TornAll, Rot, Misdirected, Failed, Every];

    fn faults(self) -> Faults {
        let torn = Faults {
            torn_p: 0.3,
            ..Faults::none(512)
        };
        match self {
            Torn => Faults {
                torn_p: 0.5,
                ..torn
            },
            TornAll => Faults {
                torn_p: 1.0,
                ..torn
            },
            Rot => Faults {
                corrupt_p: 0.004,
                latent_p: 0.002,
                ..torn
            },
            Misdirected => Faults {
                misdirect_p: 0.004,
                lost_write_p: 0.004,
                ..torn
            },
            Failed => Faults {
                fail_p: 0.004,
                ..torn
            },
            Every => Faults {
                corrupt_p: 0.002,
                latent_p: 0.001,
                misdirect_p: 0.002,
                lost_write_p: 0.002,
                fail_p: 0.002,
                ..torn
            },
        }
    }

    /// Returns whether the fault may damage the file.
    fn damages(self) -> bool {
        matches!(self, Rot | Misdirected | Every)
    }

    /// Returns the strikes of the fault `t` counted.
    fn struck(self, t: &Tally) -> usize {
        match self {
            Torn | TornAll => t.torn,
            Failed => t.failed,
            Rot | Misdirected | Every => t.damage,
        }
    }
}

/// What the crash table runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Device {
    Simulated,
    Pool,
    Uring,
}

#[derive(Debug, Default)]
struct Tally {
    seeds: usize,
    crashes: usize,
    acked: usize,
    pages: usize,
    logs: usize,
    reopened: usize,
    unopenable: usize,
    /// Unforced writes a crash tore.
    torn: usize,
    /// The writes, forces and allocations the faults failed.
    failed: usize,
    /// Findings, refused page reads and refused opens.
    damage: usize,
    /// Each acknowledged commit a round allowed lost, by seed, round and id.
    lost: Vec<(u64, usize, u64)>,
}

impl Tally {
    /// Count what `report`, reopened after a crash, found and whether it opened.
    fn count(&mut self, report: &Report) {
        self.damage +=
            report.findings.len() + report.pages.values().filter(|p| p.is_none()).count();
        for f in &report.findings {
            match f {
                Finding::Page { .. } => self.pages += 1,
                Finding::Log(_) => self.logs += 1,
                Finding::Orphan(_) | Finding::Unlisted => {}
            }
        }
        if report.open.is_some() {
            self.unopenable += 1;
            self.damage += 1;
        } else {
            self.reopened += 1;
        }
    }

    fn count_acked(&mut self, client: &Client, ledger: &Ledger) {
        self.acked += client
            .world
            .lock()
            .unwrap()
            .writes
            .keys()
            .filter(|&&x| ledger.fate(x) == Some(Fate::Acknowledged))
            .count();
    }
}

/// Returns the findings `rate` asks of `seeds` seeds run, at least one unless `rate` asks none.
fn kind_floor(seeds: usize, rate: &Rate) -> usize {
    (seeds * rate.found / rate.per).max(rate.found.min(1))
}

/// Run `seeds` of `fault` under `shape` on `device`, failing unless the fault struck, the row
/// reached `floors` and it lost an acknowledged write only to faults that damage the file.
fn crash_row(device: Device, fault: Fault, shape: Shape, seeds: Range<u64>, floors: Floors) {
    let row = (device, fault, shape, seeds.clone());
    let mut t = Tally::default();
    for seed in sweep(seeds.clone()) {
        t.seeds += 1;
        match device {
            Simulated => crash_rounds(&mut t, seed, fault, shape.layout()),
            Pool => os_rounds(&mut t, Backend::Pool { threads: 4 }, fault, shape, seed),
            Uring => os_rounds(&mut t, Backend::Uring { entries: 64 }, fault, shape, seed),
        }
    }
    let f = &floors;
    let reach = |what: &str, count: usize, floor: usize| {
        assert_reached(&format!("{row:?} {t:?}: {what}"), count, floor);
    };
    reach(
        "strikes",
        fault.struck(&t),
        share(seeds.clone(), f.struck).max(1),
    );
    reach("crashes", t.crashes, share(seeds.clone(), f.crashes));
    reach(
        "acknowledged writes",
        t.acked,
        share(seeds.clone(), f.acked),
    );
    reach("reopens", t.reopened, share(seeds, f.reopened));
    if fault.damages() {
        assert!(
            t.unopenable <= t.crashes.div_ceil(CRASHES_PER_REFUSAL),
            "{row:?}: {} refused opens over {} crashes",
            t.unopenable,
            t.crashes
        );
    } else {
        assert_eq!(
            Vec::<(u64, usize, u64)>::new(),
            t.lost,
            "{row:?}: no fault of the row loses a write or damages the file"
        );
    }
    if every_case() {
        reach("page findings", t.pages, kind_floor(t.seeds, &f.pages));
        reach("log findings", t.logs, kind_floor(t.seeds, &f.logs));
    }
}

/// Crash a random workload four times under `fault`, reopening and checking after each.
fn crash_rounds(tally: &mut Tally, seed: u64, fault: Fault, layout: Layout) {
    let quiet = Faults {
        atomic_unit: if layout.atomic { PAGE as u64 } else { 512 },
        ..Faults::none(512)
    };
    let faults = Faults {
        atomic_unit: quiet.atomic_unit,
        ..fault.faults()
    };
    let mut sim = Sim::new(
        seed,
        Config {
            faults: quiet,
            ..Config::default()
        },
    );
    let client = Client::new();
    setup(&mut sim, &client, layout);
    let world = client.world.clone();
    let lost_log_write = Cell::new(false);
    let out = crash_loop(
        &mut sim,
        seed,
        faults,
        Rounds::new(4, 500, 60_000),
        |r| {
            let c = Client {
                ledger: r.ledger().clone(),
                world: world.clone(),
            };
            r.crash(|rt| drive(rt.clone(), &Host::sim(&rt), &c, layout, 400));
            lost_log_write.set(
                r.sim()
                    .trace()
                    .lines()
                    .any(|l| l.starts_with("fault lost write") && l.contains("-log/")),
            );
        },
        |rt, ledger| {
            let pages = world.lock().unwrap().pages.clone();
            let mut report = inspect(rt, &pages, layout);
            report.damage_allowed = fault.damages();
            judge(
                &mut report,
                ledger,
                &world.lock().unwrap(),
                lost_log_write.get(),
            );
            report
        },
    );
    for line in sim.trace().lines() {
        tally.torn += usize::from(line.starts_with("crash torn"));
        tally.failed += usize::from(line.starts_with("fault fail"));
    }
    for (round, v) in out.allowed() {
        if let Violation::Lost(x) = v {
            tally.lost.push((seed, round, *x));
        }
    }
    for round in out.rounds() {
        tally.crashes += usize::from(round.crashed());
        tally.count(round.reopened());
    }
    tally.count_acked(&client, out.ledger());
}

/// The sector and atomic unit of a real device.
const SECTOR: u64 = 4096;

/// Run six rounds of a random workload under `fault` and `shape` on real files, each ending in a
/// crash of the device and a reopen with no faults that is checked as [`crash_rounds`] checks it,
/// allowing the loss of an acknowledged commit under lost writes.
fn os_rounds(tally: &mut Tally, backend: Backend, fault: Fault, shape: Shape, seed: u64) {
    const ROUNDS: u64 = 6;
    silence_fatal_panics();
    let layout = Layout {
        log_block: SECTOR as u32,
        ..shape.layout()
    };
    let faults = fault.faults();
    let held = Scratch::new("pager", &format!("{fault:?}-{shape:?}-{seed}"));
    let dir = held.path();
    let client = Client::new();
    create(OsRuntime::new(dir, backend), &client, layout);
    for round in 0..ROUNDS {
        let rt = OsRuntime::with_faults(
            dir,
            backend,
            Faults {
                seed: seed * ROUNDS + round,
                sector: SECTOR,
                atomic_unit: SECTOR,
                ..faults
            },
        );
        let host = Host::seeded(seed * ROUNDS + round);
        let steps = 20 + (host.draw() % 60) as usize;
        let run = panic::catch_unwind(AssertUnwindSafe(|| {
            drive(rt.clone(), &host, &client, layout, steps);
        }));
        if let Err(payload) = run {
            let Some(failure) = payload.downcast_ref::<FatalIo>() else {
                panic::resume_unwind(payload)
            };
            assert_eq!(
                IoError::Failed,
                failure.cause(),
                "seed {seed} round {round}: {failure}"
            );
            tally.failed += 1;
        }
        tally.torn += rt.crash().torn;
        tally.crashes += 1;
        let pages = client.world.lock().unwrap().pages.clone();
        let mut report = inspect(OsRuntime::new(dir, backend), &pages, layout);
        report.damage_allowed = fault.damages();
        judge(
            &mut report,
            &client.ledger,
            &client.world.lock().unwrap(),
            faults.lost_write_p > 0.0,
        );
        let (allowed, violations): (Vec<Violation>, Vec<Violation>) =
            check(&client.ledger, &report)
                .into_iter()
                .partition(|v| report.allows(v));
        assert_eq!(
            Vec::<Violation>::new(),
            violations,
            "{fault:?} {shape:?} seed {seed} round {round}"
        );
        for v in allowed {
            if let Violation::Lost(x) = v {
                tally.lost.push((seed, round as usize, x));
            }
        }
        report.settle(&client.ledger);
        tally.count(&report);
        if report.last() {
            break;
        }
    }
    tally.count_acked(&client, &client.ledger);
}

/// Run one seed of every fault under every shape `device` runs, each crashing and reopening in
/// every round unless the fault damages the file.
///
/// A real device runs no failing fault under threads: the failure ends the host process from
/// the worker thread it reaches. Torn writes run only on the simulator without atomic page writes,
/// where a write can be wider than its atomic unit.
fn every_cell(device: Device) {
    let (rounds, shapes) = match device {
        Simulated => (4, &Shape::ALL[..]),
        Pool | Uring => (6, &[Plain, Atomic, Threaded][..]),
    };
    for &shape in shapes {
        for fault in Fault::ALL {
            let real = device != Simulated;
            let tears = matches!(fault, Torn | TornAll);
            if real && shape.layout().threads > 0 && fault.faults().fail_p > 0.0
                || tears && (real || shape.layout().atomic)
            {
                continue;
            }
            let start = 10_000 + 1000 * device as u64 + 100 * shape as u64 + 10 * fault as u64;
            let whole = usize::from(!fault.damages());
            let floors = floors(1.max(whole * rounds), 15, whole * rounds);
            crash_row(device, fault, shape, start..start + 1, floors);
        }
    }
}

#[test]
fn every_cell_crashes_on_the_simulator() {
    every_cell(Simulated);
}

#[test]
fn every_cell_crashes_on_a_thread_pool() {
    every_cell(Pool);
}

#[test]
fn every_cell_crashes_on_io_uring() {
    if adtb_testkit::backends::uring(64).is_some() {
        every_cell(Uring);
    }
}

#[test]
fn crashes_with_torn_writes_lose_nothing_and_find_nothing() {
    crash_row(Simulated, Torn, Plain, 0..12, floors(21, 201, 0));
}

#[test]
fn torn_log_blocks_lose_no_acknowledged_commit() {
    crash_row(Simulated, TornAll, WideLog, 600..612, floors(21, 201, 0));
}

#[test]
fn crashes_with_atomic_page_writes_lose_nothing() {
    crash_row(Simulated, Failed, Atomic, 100..106, floors(11, 0, 0));
}

#[test]
fn a_failed_write_under_atomic_page_writes_loses_nothing() {
    crash_row(Simulated, Failed, Atomic, 10257..10258, floors(1, 0, 0));
}

#[test]
fn a_failed_write_of_threads_under_atomic_page_writes_loses_nothing() {
    crash_row(Simulated, Failed, Everything, 10451..10452, floors(1, 0, 0));
}

#[test]
fn bit_rot_and_latent_errors_are_reported_never_returned() {
    crash_row(Simulated, Rot, Plain, 200..212, floors(0, 0, 0));
}

#[test]
fn misdirected_and_lost_writes_are_reported_never_returned() {
    let floors = Floors {
        pages: MISDIRECTED_PAGES,
        logs: MISDIRECTED_LOGS,
        ..floors(0, 0, 0)
    };
    crash_row(Simulated, Misdirected, Plain, 300..312, floors);
}

#[test]
fn failed_writes_and_forces_crash_and_recover() {
    crash_row(Simulated, Failed, Plain, 400..412, floors(21, 0, 0));
}

#[test]
fn every_fault_at_once() {
    let floors = Floors {
        pages: EVERY_FAULT_PAGES,
        ..floors(25, 300, 33)
    };
    crash_row(Simulated, Every, Plain, 500..512, floors);
}

#[test]
fn threads_sharing_a_pager_lose_nothing_across_crashes() {
    crash_row(Simulated, Torn, Threaded, 700..710, floors(11, 101, 0));
}

#[test]
fn threads_sharing_a_pager_under_every_fault() {
    let floors = Floors {
        pages: THREADED_PAGES,
        logs: THREADED_LOGS,
        ..floors(15, 1000, THREADED_REOPENS)
    };
    crash_row(Simulated, Every, Threaded, 800..808, floors);
}

#[test]
fn a_failed_write_or_force_on_a_thread_pool_crashes_and_recovery_keeps_every_commit() {
    let floors = Floors {
        struck: 6,
        ..floors(6, 0, 0)
    };
    crash_row(Pool, Failed, Plain, 0..2, floors);
}

#[test]
fn a_failed_write_or_force_on_io_uring_crashes_and_recovery_keeps_every_commit() {
    let floors = Floors {
        struck: 6,
        ..floors(6, 0, 0)
    };
    if adtb_testkit::backends::uring(64).is_some() {
        crash_row(Uring, Failed, Plain, 0..2, floors);
    }
}

#[test]
fn rot_latent_errors_and_misdirected_and_lost_writes_on_real_files_are_refused_never_returned() {
    crash_row(Pool, Every, Plain, 0..2, floors(1, 0, 0));
}

#[test]
fn crash_replay_at_every_flush_boundary() {
    let shapes: &[Shape] = if every_case() { &Shape::ALL } else { &[Plain] };
    for layout in shapes.iter().map(|s| s.layout()) {
        let mut sim = Sim::new(11, Config::default());
        let client = Client::new();
        setup(&mut sim, &client, layout);
        sim.record();
        let c = client.clone();
        assert_eq!(
            Outcome::Finished,
            sim.run(|rt| drive(rt.clone(), &Host::sim(&rt), &c, layout, 60))
        );
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
                            _ if client.ledger.fate(xid) == Some(Fate::Aborted) => {
                                ledger.abort(xid);
                            }
                            _ => {}
                        }
                    }
                }
                let mut replay = Sim::with_files(at as u64, Config::default(), files);
                let mut report = Report::default();
                assert_eq!(
                    Outcome::Finished,
                    replay.run(|rt| report = inspect(rt, &world.pages, layout))
                );
                let problems = judged(report, &ledger, &world);
                assert!(
                    problems.is_empty(),
                    "{layout:?} flush boundary {at}: {problems:#?}"
                );
            }
        }
        let flushes = recording.flushes().len();
        assert!(
            flushes > 20 && images > 100,
            "{layout:?}: {flushes} flushes, {images} images"
        );
    }
}

#[test]
fn threads_finish_and_every_acknowledged_write_stays() {
    let shapes: &[Shape] = if every_case() {
        &Shape::ALL
    } else {
        &[Threaded]
    };
    for layout in shapes.iter().map(|s| s.layout()) {
        for seed in sweep(900..904) {
            let mut sim = Sim::new(seed, Config::default());
            let client = Client::new();
            setup(&mut sim, &client, layout);
            let c = client.clone();
            assert_eq!(
                Outcome::Finished,
                sim.run(|rt| drive(rt.clone(), &Host::sim(&rt), &c, layout, 320))
            );
            let pages = client.world.lock().unwrap().pages.clone();
            let mut report = Report::default();
            assert_eq!(
                Outcome::Finished,
                sim.run(|rt| report = inspect(rt, &pages, layout))
            );
            let world = client.world.lock().unwrap();
            let problems = judged(report, &client.ledger, &world);
            assert!(problems.is_empty(), "seed {seed}: {problems:#?}");
            let acked = world
                .writes
                .keys()
                .filter(|&&x| client.ledger.fate(x) == Some(Fate::Acknowledged))
                .count();
            assert_eq!(
                world.writes.len(),
                acked,
                "{layout:?} seed {seed}: every transaction ends acknowledged"
            );
        }
    }
}

#[test]
fn a_failed_log_allocate_is_fatal_and_leaves_a_file_a_later_create_can_use() {
    silence_fatal_panics();
    let mut failed = false;
    for seed in 0..64 {
        let mut sim = Sim::new(
            seed,
            Config {
                faults: Faults {
                    fail_p: 0.2,
                    ..Faults::none(512)
                },
                ..Config::default()
            },
        );
        let outcome = run_process(&mut sim, |rt| {
            if let Err(e) = Pager::create(rt, &options(PLAIN)) {
                assert_eq!(OpenError::Io(IoError::Failed), e, "seed {seed}");
            }
        });
        if sim.trace().contains("fault fail allocate") {
            failed = true;
            assert_eq!(Outcome::Crashed, outcome, "seed {seed}");
            sim.set_faults(Faults::none(512));
            let outcome = sim.run(|rt| {
                Pager::create(rt, &options(PLAIN)).expect("create over what the failure left");
            });
            assert_eq!(Outcome::Finished, outcome, "seed {seed}");
        }
    }
    assert!(failed, "no allocate failed on seeds 0..64");
}

#[test]
fn a_seed_replays_exactly() {
    let run = |seed| {
        let mut sim = Sim::new(seed, Config::default());
        let client = Client::new();
        setup(&mut sim, &client, PLAIN);
        sim.crash_at(sim.now() + Duration::from_millis(20));
        let c = client.clone();
        run_process(&mut sim, |rt| {
            workload(rt.clone(), &Host::sim(&rt), &c, PLAIN, 200);
        });
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

/// Returns a copy of `sim`'s device after a clean reopen, with byte `at` of the body of page `pid`
/// inverted.
fn flipped(sim: &mut Sim, pid: Pid, at: usize) -> Sim {
    with_pager(sim, |_| ());
    let mut files = sim.files();
    DATA.flip(&mut files, pid, at);
    Sim::with_files(1, Config::default(), files)
}

/// Returns a device holding a database whose page `pid` carries an LSN past the end of its log,
/// with the LSN of the last record that wrote the page and the LSN it carries.
fn page_past_log_end() -> (Sim, Pid, Lsn, Lsn) {
    page_changed_past_log_end(PLAIN, false)
}

/// Returns what [`page_past_log_end`] does, after an open under `layout` that, when `change`,
/// changes the page and writes it out with no close.
fn page_changed_past_log_end(layout: Layout, change: bool) -> (Sim, Pid, Lsn, Lsn) {
    let mut sim = Sim::new(3, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let pid = client.world.lock().unwrap().pages[3];
    let outcome = sim.run(|rt| {
        let (p, _) = Pager::open(rt, &options(layout)).expect("open");
        if change {
            p.update(&[pid], |v| v[0][HEADER_LEN] ^= 1).unwrap();
            p.flush_log();
            p.flush_pages(usize::MAX);
        }
    });
    assert_eq!(Outcome::Finished, outcome);
    let files = sim.files();
    let written = crate::storage::lsn(DATA.page(&files, pid));
    let past = Lsn(1 << 40);
    (planted(files, pid, past), pid, written, past)
}

/// Returns a device holding `files` with page `pid` resealed at LSN `lsn`.
fn planted(mut files: BTreeMap<String, Vec<u8>>, pid: Pid, lsn: Lsn) -> Sim {
    let page = DATA.page_mut(&mut files, pid);
    page[..8].copy_from_slice(&lsn.0.to_le_bytes());
    crate::storage::seal(pid, page);
    Sim::with_files(1, Config::default(), files)
}

/// Returns the end of the log recovery kept on opening `sim`, and what `body` then returns.
fn opened<T>(sim: &mut Sim, body: impl FnOnce(&Pager<SimRuntime>) -> T) -> (Lsn, T) {
    let mut out = None;
    let outcome = sim.run(|rt| {
        let (p, opening) = Pager::open(rt, &options(PLAIN)).expect("open");
        let recovery = opening.recovery().cloned().expect("the open recovered");
        out = Some((recovery.end, body(&p)));
    });
    assert_eq!(Outcome::Finished, outcome);
    out.unwrap()
}

fn unlogged(lsn: Lsn, pid: Pid, page: Lsn) -> Finding {
    Finding::Log(LogFinding {
        lsn,
        problem: LogProblem::Unlogged { pid, page },
    })
}

/// Returns the findings of a page `pid` the file holds at `found`, at or past `end`.
fn past_end(pid: Pid, end: Lsn, found: Lsn) -> [Finding; 2] {
    let damage = Damage::Stale {
        expected: end,
        found,
    };
    [Finding::Page { pid, damage }, unlogged(end, pid, found)]
}

#[test]
fn a_page_past_the_log_end_is_reported_as_stale_and_an_unlogged_write() {
    let (mut sim, pid, _, past) = page_past_log_end();
    let (end, findings) = opened(&mut sim, |p| p.verify().findings);
    assert_eq!(past_end(pid, end, past).to_vec(), findings);
}

#[test]
fn replay_refuses_a_page_past_the_log_end_whose_change_it_redoes() {
    let atomic = Layout {
        atomic: true,
        ..PLAIN
    };
    let (mut sim, pid, _, past) = page_changed_past_log_end(atomic, true);
    let mut out = None;
    let outcome = sim.run(|rt| {
        let (p, opening) = Pager::open(rt, &options(atomic)).expect("open");
        let recovery = opening.recovery().cloned().expect("the open recovered");
        let read = p.read(pid).map(|_| ()).map_err(|e| e.damage);
        out = Some((recovery.end, recovery.damaged, read));
    });
    assert_eq!(Outcome::Finished, outcome);
    let (end, damaged, read) = out.unwrap();
    let stale = Damage::Stale {
        expected: end,
        found: past,
    };
    assert_eq!(vec![(pid, stale)], damaged);
    assert_eq!(Err(stale), read);
}

#[test]
fn replay_heals_a_page_past_the_log_end_from_its_full_page_image() {
    let (mut sim, pid, _, _) = page_changed_past_log_end(PLAIN, true);
    let (_, (damaged, read)) = opened(&mut sim, |p| {
        (Probe::of(p).refusals(), p.read(pid).map(|_| ()))
    });
    assert_eq!(BTreeMap::new(), damaged);
    assert_eq!(Ok(()), read);
}

#[test]
fn a_page_one_below_the_recovered_end_reads_and_one_at_it_is_refused() {
    let (sim, pid, _, _) = page_past_log_end();
    let files = sim.files();
    let (end, ()) = opened(
        &mut Sim::with_files(1, Config::default(), files.clone()),
        |_| (),
    );
    let damage = Damage::Stale {
        expected: end,
        found: end,
    };
    for (lsn, want) in [
        (Lsn(end.0 - 1), Ok(())),
        (end, Err(PageError { pid, damage })),
    ] {
        let (at, read) = opened(&mut planted(files.clone(), pid, lsn), |p| {
            p.read(pid).map(|_| ())
        });
        assert_eq!(end, at);
        assert_eq!(want, read, "a page at {lsn:?}");
    }
}

#[test]
fn a_page_whose_only_records_are_in_the_lost_log_tail_is_an_unlogged_write() {
    let mut sim = Sim::new(4, Config::default());
    setup(&mut sim, &Client::new(), PLAIN);
    let (cut, pid) = with_pager(&mut sim, |p| {
        p.flush_log();
        let cut = Probe::of(p).durable();
        while Probe::of(p).appended().0 < cut.0 + 16 * PAGE as u64 {
            p.reserve_xids(1);
        }
        let pid = p.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        p.update(&[pid], |v| v[0][HEADER_LEN] ^= 1).unwrap();
        p.flush_log();
        p.flush_pages(usize::MAX);
        (cut, pid)
    });
    let mut files = sim.files();
    let dir = format!("{}/", options(PLAIN).log_dir());
    let segment = options(PLAIN).segment_size;
    for (path, bytes) in files.iter_mut().filter(|(path, _)| path.starts_with(&dir)) {
        let base = u64::from_str_radix(&path[dir.len()..], 16).unwrap();
        let from = cut.0.saturating_sub(base).min(segment) as usize;
        bytes[from..].fill(0);
    }
    let pages = (DATA.bytes(&files).len() / PAGE) as u64;
    let past: Vec<(Pid, Lsn)> = (Pid::FIRST_DATA.0..pages)
        .map(|n| (Pid(n), crate::storage::lsn(DATA.page(&files, Pid(n)))))
        .filter(|&(_, lsn)| lsn >= cut)
        .collect();
    assert!(past.iter().any(|&(p, _)| p == pid), "{past:?}");
    let mut sim = Sim::with_files(1, Config::default(), files.clone());
    let (end, findings) = opened(&mut sim, |p| p.verify().findings);
    assert_eq!(cut, end);
    let mut expected: Vec<Finding> = past
        .iter()
        .flat_map(|&(p, l)| past_end(p, cut, l))
        .collect();
    expected.sort();
    assert_eq!(expected, findings);
    let stale = |&(pid, found): &(Pid, Lsn)| {
        let damage = Damage::Stale {
            expected: cut,
            found,
        };
        (pid, found, Err(PageError { pid, damage }))
    };
    let refused: Vec<_> = past.iter().map(stale).collect();
    let highest = past.iter().map(|&(_, l)| l).max().unwrap();
    for append_past in [false, true] {
        let mut sim = Sim::with_files(1, Config::default(), files.clone());
        let (_, read) = opened(&mut sim, |p| {
            let mut appends = 0;
            while append_past && Probe::of(p).appended() <= highest {
                p.reserve_xids(1);
                appends += 1;
                assert!(appends < 100_000, "the log passes LSN {highest:?}");
            }
            let next = Probe::of(p).appended();
            let read = |&(pid, found): &(Pid, Lsn)| (pid, found, p.read(pid).map(|_| ()));
            let read: Vec<_> = past.iter().map(read).collect();
            assert!(
                past.iter().all(|&(_, l)| (l < next) == append_past),
                "{next:?}"
            );
            read
        });
        assert_eq!(refused, read, "appended past every page: {append_past}");
    }
}

#[test]
fn a_change_to_a_page_past_the_log_end_is_refused_and_reported_on_every_verify() {
    let (mut sim, pid, _, past) = page_past_log_end();
    let planted = sim.files();
    let (end, refused) = opened(&mut sim, |p| {
        let refused = p.update(&[pid], |v| v[0][HEADER_LEN] ^= 1);
        refused.map(|_| ()).map_err(|e| e.damage)
    });
    let [stale, _] = past_end(pid, end, past);
    assert_eq!(
        Finding::Page {
            pid,
            damage: refused.unwrap_err()
        },
        stale
    );
    let (end, (first, again)) = opened(&mut sim, |p| (p.verify().findings, p.verify().findings));
    assert_eq!(past_end(pid, end, past).to_vec(), first);
    assert_eq!(first, again);
    assert_eq!(DATA.page(&planted, pid), DATA.page(&sim.files(), pid));
}

#[test]
fn a_rotted_page_is_refused_and_reported() {
    let mut sim = Sim::new(1, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let pid = client.world.lock().unwrap().pages[3];
    let mut sim = flipped(&mut sim, pid, 1000 - HEADER_LEN);
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
    let mut sim = flipped(&mut sim, Pid(2), 500 - HEADER_LEN);
    let got = with_pager(&mut sim, |p| p.allocate(1, PageKind::HeapSlotted, 7));
    assert_eq!(
        Pid(2),
        got.expect_err("allocation used a damaged bitmap").pid
    );
}

#[test]
fn allocation_reuses_a_damaged_free_page_without_reading_it() {
    let mut sim = Sim::new(3, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let last = *client.world.lock().unwrap().pages.last().unwrap();
    let next = Pid(last.0 + 1);
    let mut files = sim.files();
    assert!(
        DATA.bytes(&files).len() <= DATA.range(next).start,
        "page {next:?} is past the file's end"
    );
    DATA.extend(&mut files, next.0 as usize + 1);
    DATA.page_mut(&mut files, next)[77] = 1;
    let mut sim = Sim::with_files(3, Config::default(), files);
    let got = with_pager(&mut sim, |p| {
        let pid = p.allocate(1, PageKind::HeapSlotted, 7);
        let read = p
            .read(next)
            .map(|g| (crate::storage::kind(&g), g.owner(), g[77]));
        let of_next = |f: &Finding| matches!(f, Finding::Page { pid, .. } if *pid == next);
        let found: Vec<Finding> = p.verify().findings.into_iter().filter(of_next).collect();
        (pid, read, found)
    });
    let formatted = (PageKind::HeapSlotted as u16, 7, 0);
    assert_eq!((Ok(next), Ok(formatted), Vec::new()), got);
}

#[test]
fn verify_reports_the_damage_of_a_page_whose_bitmap_page_the_device_cannot_read() {
    let mut sim = Sim::new(3, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let p = client.world.lock().unwrap().pages[0];
    let mut sim = flipped(&mut sim, p, 0);
    let bitmap = Pid(2);
    let mut found = None;
    let outcome = sim.run(|rt| {
        let reads = Match::kind(OpKind::Read).offset(DATA.range(bitmap).start as u64);
        rt.plan(Plan::new().rule(reads, Action::Fail(IoError::Failed)));
        let (pager, _) = Pager::open(rt, &options(PLAIN)).expect("open");
        found = Some(pager.verify().findings);
    });
    assert_eq!(Outcome::Finished, outcome);
    let unreadable = Finding::Page {
        pid: bitmap,
        damage: Damage::Unreadable,
    };
    let rotted = Finding::Page {
        pid: p,
        damage: Damage::Checksum,
    };
    assert_eq!(Some(vec![unreadable, rotted]), found);
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
                    ..Faults::none(512)
                },
                ..Config::default()
            },
        );
        let client = Client::new();
        sim.set_faults(Faults::none(512));
        setup(&mut sim, &client, PLAIN);
        let durable: Arc<Mutex<BTreeSet<Pid>>> = Arc::default();
        let faults = Faults {
            torn_p: 0.5,
            fail_p: 0.01,
            ..Faults::none(512)
        };
        crash_loop(
            &mut sim,
            seed,
            faults,
            Rounds::new(4, 300, 9000),
            |r| {
                let d = durable.clone();
                r.crash(|rt| {
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
            },
            |rt, _| {
                let (p, _) = Pager::open(rt, &options(PLAIN)).expect("open");
                let mut problems: Vec<String> = p
                    .verify()
                    .findings
                    .iter()
                    .map(|f| format!("{f:?}"))
                    .collect();
                let fresh = p.allocate(1, PageKind::HeapSlotted, 9).expect("allocate");
                if durable.lock().unwrap().contains(&fresh) {
                    problems.push(format!("{fresh:?} handed out twice"));
                }
                Reopen::found(problems)
            },
        );
    }
}

/// A latch a thread takes on a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Latch {
    Shared,
    Exclusive,
    Optimistic,
    /// An update, which takes the exclusive latch for its change.
    Update,
}

impl Latch {
    const ALL: [Latch; 4] = [
        Latch::Shared,
        Latch::Exclusive,
        Latch::Optimistic,
        Latch::Update,
    ];

    /// Take this latch on `pid` and give it back.
    fn take(self, p: &Pager<SimRuntime>, pid: Pid) {
        match self {
            Latch::Shared => drop(p.read(pid).unwrap()),
            Latch::Exclusive => drop(p.exclusive(pid).unwrap()),
            Latch::Optimistic => assert!(p.optimistic(pid).unwrap().validate()),
            Latch::Update => {
                p.update(&[pid], |_| {}).unwrap();
            }
        }
    }
}

/// What a thread holds on a page when it asks for a latch on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    Shared,
    Exclusive,
    /// A shared and then an exclusive latch, each given back.
    Released,
}

/// Returns the page and the panic message of asking for `call` on it while holding `held` on it,
/// or `None` when the call returns.
fn latch_again(held: Held, call: Latch) -> (Pid, Option<String>) {
    let mut sim = Sim::new(21, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let pid = client.world.lock().unwrap().pages[0];
    let caught = panic::catch_unwind(AssertUnwindSafe(|| {
        with_pager(&mut sim, |p| match held {
            Held::Shared => {
                let guard = p.read(pid).unwrap();
                call.take(p, pid);
                drop(guard);
            }
            Held::Exclusive => {
                let guard = p.exclusive(pid).unwrap();
                call.take(p, pid);
                drop(guard);
            }
            Held::Released => {
                Latch::Shared.take(p, pid);
                Latch::Exclusive.take(p, pid);
                call.take(p, pid);
            }
        });
    }));
    let message = caught.err().map(|payload| {
        payload
            .downcast_ref::<String>()
            .cloned()
            .unwrap_or_else(|| format!("{call:?} under {held:?} panicked with no message"))
    });
    (pid, message)
}

#[test]
fn a_thread_latching_a_page_it_holds_panics_naming_the_page_except_optimistic_beside_shared() {
    let mut got = Vec::new();
    let mut want = Vec::new();
    for held in [Held::Shared, Held::Exclusive, Held::Released] {
        for call in Latch::ALL {
            let (pid, message) = latch_again(held, call);
            got.push((held, call, message));
            let allowed =
                held == Held::Released || (held, call) == (Held::Shared, Latch::Optimistic);
            let named = format!("page {} is already latched by this thread", pid.0);
            want.push((held, call, (!allowed).then_some(named)));
        }
    }
    assert_eq!(12, got.len());
    assert_eq!(want, got);
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
    const SEEDS: Range<u64> = 0..10;
    let mut leaks_possible_before = 0;
    for seed in sweep(SEEDS) {
        let mut sim = Sim::new(seed, Config::default());
        let client = Client::new();
        setup(&mut sim, &client, PLAIN);
        let index = client.world.lock().unwrap().pages[0];
        let before = with_pager(&mut sim, |p| allocated(p));
        let faults = Faults {
            torn_p: 0.5,
            ..Faults::none(512)
        };
        crash_loop(
            &mut sim,
            seed,
            faults,
            Rounds::new(4, 20_000, 80_000),
            |r| {
                r.crash(|rt| {
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
            },
            |rt, _| {
                let (p, _) = Pager::open(rt, &options(PLAIN)).expect("open");
                let fresh: BTreeSet<Pid> = allocated(&p).difference(&before).copied().collect();
                let links = linked(&p, index);
                let mut problems = Vec::new();
                if fresh != links {
                    problems.push(format!("allocations {fresh:?} and links {links:?} differ"));
                }
                for &pid in &links {
                    let page = p.read(pid).unwrap();
                    let made = crate::storage::kind(&page) == PageKind::HeapSlotted as u16
                        && page.owner() == 9
                        && page[HEADER_LEN] == 0xab
                        && page[HEADER_LEN + 1..].iter().all(|&b| b == 0);
                    if !made {
                        problems.push(format!("{pid:?} is not the page its link was made with"));
                    }
                }
                Reopen::found(problems)
            },
        );
    }
    assert!(
        leaks_possible_before > share(SEEDS, 50),
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
            crate::storage::kind(&new.image(pid))
        );
        let mut parent = p.exclusive(index).unwrap();
        let mut child = new.latch(pid);
        let before = Probe::of(p).durable();
        p.update_all_with(new, &mut [&mut parent, &mut child], |pages| {
            pages[0][HEADER_LEN..HEADER_LEN + 8].copy_from_slice(&pid.0.to_le_bytes());
            pages[1][HEADER_LEN] = 1;
        })
        .unwrap();
        drop((parent, child));
        assert_eq!(
            before,
            Probe::of(p).durable(),
            "nothing is flushed until asked"
        );
        p.flush_log();
        (pid, allocated(p))
    });
    assert!(lsns.contains(&pid));
    with_pager(&mut sim, |p| {
        assert_eq!(BTreeSet::from([pid]), linked(p, index));
        assert_eq!(3, crate::storage::format(&p.read(pid).unwrap()));
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
        p.update_with(new, &[index], |_| {}).unwrap();
    });
}

#[test]
fn a_damaged_bitmap_fails_a_reservation() {
    let mut sim = Sim::new(34, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let mut sim = flipped(&mut sim, Pid(2), 500 - HEADER_LEN);
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
            ..Faults::none(512)
        });
        p.flush_pages(usize::MAX);
        rt.set_faults(Faults::none(512));
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

/// Write every page of `pages`, then change the first and flush it under `faults`. Returns the
/// LSN the change gave it and the LSN the device held for it before.
fn flush_first_under(
    rt: &SimRuntime,
    p: &Pager<SimRuntime>,
    pages: &[Pid],
    faults: Faults,
) -> (Lsn, Lsn) {
    for &pid in pages {
        p.update(&[pid], |v| v[0][HEADER_LEN] ^= 1).unwrap();
    }
    p.flush_pages(usize::MAX);
    let before = crate::storage::lsn(&p.read(pages[0]).unwrap());
    let changed = p.update(&[pages[0]], |v| v[0][HEADER_LEN + 1] = 7).unwrap();
    p.flush_log();
    rt.set_faults(faults);
    p.flush_pages(usize::MAX);
    rt.set_faults(Faults {
        sector: faults.sector,
        atomic_unit: faults.atomic_unit,
        ..Faults::none(512)
    });
    (changed, before)
}

/// Returns the page the first misdirected write to the data file in `trace` landed on.
fn misdirected_to(trace: &str) -> Pid {
    let line = trace
        .lines()
        .find(|l| l.starts_with("fault misdirect") && l.contains(NAME))
        .expect("no write to the data file was misdirected");
    let at: u64 = line.rsplit_once('@').unwrap().1.parse().unwrap();
    Pid(at / PAGE as u64)
}

#[test]
fn a_misdirected_page_write_is_refused_where_it_was_meant_and_where_it_landed() {
    let layout = Layout {
        log_block: PAGE as u32,
        ..PLAIN
    };
    let mut landed_on_a_read_page = false;
    for seed in 0..4 {
        let mut sim = Sim::new(
            seed,
            Config {
                faults: Faults::none(PAGE as u64),
                ..Config::default()
            },
        );
        let client = Client::new();
        setup(&mut sim, &client, layout);
        let pages = client.world.lock().unwrap().pages.clone();
        let mut seen = None;
        let outcome = sim.run(|rt| {
            let (p, _) = Pager::open(rt.clone(), &options(layout)).unwrap();
            let misdirect = Faults {
                misdirect_p: 1.0,
                ..Faults::none(PAGE as u64)
            };
            let (expected, found) = flush_first_under(&rt, &p, &pages, misdirect);
            let mut refused = BTreeMap::new();
            for _ in 0..3 {
                for &pid in &pages[1..] {
                    if let Err(e) = p.read(pid) {
                        refused.insert(pid, e.damage);
                    }
                }
            }
            let back = p.read(pages[0]).map(|_| ()).map_err(|e| e.damage);
            seen = Some((Damage::Stale { expected, found }, back, refused));
        });
        assert_eq!(Outcome::Finished, outcome, "seed {seed}");
        let (stale, back, refused) = seen.unwrap();
        assert_eq!(Err(stale), back, "seed {seed}");
        let landed = misdirected_to(&sim.trace());
        let hit: BTreeMap<Pid, Damage> = pages[1..]
            .contains(&landed)
            .then_some((landed, Damage::Checksum))
            .into_iter()
            .collect();
        landed_on_a_read_page |= !hit.is_empty();
        assert_eq!(hit, refused, "seed {seed}");
    }
    assert!(
        landed_on_a_read_page,
        "no misdirected write landed on a page read back"
    );
}

#[test]
fn a_lost_page_write_is_reported_and_refused_on_reopen() {
    let mut sim = Sim::new(41, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let pages = client.world.lock().unwrap().pages.clone();
    let mut lsns = None;
    let outcome = sim.run(|rt| {
        let (p, _) = Pager::open(rt.clone(), &options(PLAIN)).unwrap();
        let lost = Faults {
            lost_write_p: 1.0,
            ..Faults::none(512)
        };
        lsns = Some(flush_first_under(&rt, &p, &pages, lost));
        p.checkpoint();
    });
    assert_eq!(Outcome::Finished, outcome);
    let (expected, found) = lsns.unwrap();
    let stale = Damage::Stale { expected, found };
    let (findings, back) = with_pager(&mut sim, |p| {
        let findings = p.verify().findings;
        (findings, p.read(pages[0]).map(|_| ()).map_err(|e| e.damage))
    });
    assert_eq!(
        vec![Finding::Page {
            pid: pages[0],
            damage: stale
        }],
        findings
    );
    assert_eq!(Err(stale), back);
}

#[test]
fn a_lost_log_write_refuses_the_reopen_at_its_gap() {
    let mut sim = Sim::new(42, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let pages = client.world.lock().unwrap().pages.clone();
    let mut cut = None;
    let outcome = sim.run(|rt| {
        let (p, _) = Pager::open(rt.clone(), &options(PLAIN)).unwrap();
        let change = |pid| p.update(&[pid], |v| v[0][HEADER_LEN] ^= 1).unwrap();
        change(pages[1]);
        p.flush_log();
        cut = Some(Probe::of(&p).durable());
        rt.set_faults(Faults {
            lost_write_p: 1.0,
            ..Faults::none(512)
        });
        change(pages[2]);
        p.flush_log();
        rt.set_faults(Faults::none(512));
        change(pages[3]);
        p.flush_log();
    });
    assert_eq!(Outcome::Finished, outcome);
    let gap = LogFinding {
        lsn: cut.unwrap(),
        problem: LogProblem::Gap,
    };
    let outcome = sim.run(|rt| {
        assert_eq!(
            Some(OpenError::Log(gap)),
            Pager::open(rt, &options(PLAIN)).err()
        );
    });
    assert_eq!(Outcome::Finished, outcome);
}

#[test]
fn a_crash_never_separates_freeing_from_unlinking() {
    const SEEDS: Range<u64> = 0..10;
    let mut freed = 0;
    for seed in sweep(SEEDS) {
        let mut sim = Sim::new(seed + 50, Config::default());
        let client = Client::new();
        setup(&mut sim, &client, PLAIN);
        let index = client.world.lock().unwrap().pages[0];
        let before = with_pager(&mut sim, |p| allocated(p));
        let faults = Faults {
            torn_p: 0.5,
            ..Faults::none(512)
        };
        crash_loop(
            &mut sim,
            seed,
            faults,
            Rounds::new(4, 20_000, 80_000),
            |r| {
                r.crash(|rt| {
                    let Ok((p, _)) = Pager::open(rt.clone(), &options(PLAIN)) else {
                        return;
                    };
                    for _ in 0..400 {
                        let page = p.read(index).unwrap().to_vec();
                        let at = |s: usize| HEADER_LEN + s * 8;
                        let pid_at = |s: usize| {
                            u64::from_le_bytes(page[at(s)..at(s) + 8].try_into().unwrap())
                        };
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
            },
            |rt, _| {
                let (p, _) = Pager::open(rt, &options(PLAIN)).expect("open");
                let fresh: BTreeSet<Pid> = allocated(&p).difference(&before).copied().collect();
                let links = linked(&p, index);
                Reopen::found(if fresh == links {
                    Vec::new()
                } else {
                    vec![format!("allocations {fresh:?} and links {links:?} differ")]
                })
            },
        );
    }
    assert!(freed > share(SEEDS, 50), "{freed} frees");
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
        p.update_with(new, &[], |_| {}).unwrap();
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
#[should_panic(expected = "this thread latches 6 pages, more than 5")]
fn latching_more_than_the_bound_from_one_thread_panics() {
    let mut sim = Sim::new(37, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    let pages = client.world.lock().unwrap().pages.clone();
    with_pager(&mut sim, |p| {
        let held: Vec<_> = pages[..LATCHES_PER_THREAD]
            .iter()
            .map(|&pid| p.exclusive(pid).unwrap())
            .collect();
        drop(p.read(pages[LATCHES_PER_THREAD]));
        drop(held);
    });
}

#[test]
fn a_failed_scrap_leaves_the_page_damaged() {
    let mut sim = Sim::new(5, Config::default());
    let client = Client::new();
    setup(&mut sim, &client, PLAIN);
    with_pager(&mut sim, |p| {
        let pid = p.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        p.damage(pid, Damage::Checksum);
        p.damage(Pid(2), Damage::Checksum);
        assert_eq!(Pid(2), p.scrap(pid).expect_err("the bitmap is damaged").pid);
        assert_eq!(
            Some(pid),
            p.read(pid).err().map(|e| e.pid),
            "a failed scrap leaves the page refused"
        );
    });
}

#[test]
fn verify_reports_a_refused_page_past_the_end_of_the_file_and_the_log() {
    let mut sim = Sim::new(1, Config::default());
    setup(&mut sim, &Client::new(), PLAIN);
    let beyond = Pid(1 << 11);
    let findings = with_pager(&mut sim, |p| {
        p.damage(beyond, Damage::Checksum);
        p.verify().findings
    });
    assert!(findings.contains(&Finding::Page {
        pid: beyond,
        damage: Damage::Checksum,
    }));
}
