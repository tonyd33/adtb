//! The engine's answers under a grid of memory grants and spill bounds against its own answers
//! under the default grant, over drawn cases at two sizes of data and corpus cases, each query
//! read in the present and at a commit position: each query gives the same rows or is refused for
//! its grant, a refusal at a grant is one at half of it too, spill space never turns an answer
//! into a refusal, a query with spill space is refused only past what it holds with no bound, and
//! a call holds outside its grant no more than the spill buffers and the simulated disk's copies
//! of its spill past what it holds under the default grant and over half the data.

use adtb::testing::Knobs;
use adtb::{TxnCallOptions, TxnOptions};

use super::{
    Case, CreateOptions, Database, Error, Got, Limit, Log, OpenOptions, QUERIES, RejectReason, Rng,
    Sim, SimRuntime, close, got_of, process, profile, scripts, seeds, texts, write,
};
use adtb_sim::Config;
use adtb_testkit::cases::case_doubled;

/// The least grant of the grid; each grant runs against its double.
const LEAST: usize = 4096;

/// The grants of the grid, as doublings of [`LEAST`], [`profile::deep`] sized.
fn grants() -> Vec<usize> {
    let doublings = if profile::is("deep") { 6 } else { 3 };
    (0..doublings).map(|i| LEAST << i).collect()
}

/// The rows of a morsel under the grid, so that a scan's batch fits the least grant.
const MORSEL: usize = 4;

/// The bytes a call may hold outside its grant past what it holds under the default grant: a
/// chunk of 8 KiB, read and written, for each of the 64 runs one merge reads and the 16
/// partitions of a spilled build, and the allocator's rounding of the set it holds.
const SPILL_BUFFERS: usize = (64 + 16) * 2 * 8192 + (64 << 10);

/// The heap the simulated disk holds for each byte a call spills: the file's visible bytes, up
/// to twice over in their vector's growth, and the copy its unsynced write keeps.
const DISK_COPIES: usize = 3;

/// How a query of the grid ended.
#[derive(Clone, Debug, PartialEq)]
enum Ran {
    Got(Got),
    /// Refused for a memory grant of these bytes.
    Memory(i64),
}

/// One run of a query: how it ended, the most it held of its grant and the bytes it held at
/// most outside its grant.
struct Run {
    ran: Ran,
    charged: usize,
    uncharged: usize,
    spilled: usize,
}

/// A point of the grid: a grant, with the engine's spill bound or the one given.
type Point = (usize, Option<usize>);

fn points() -> Vec<Point> {
    grants()
        .into_iter()
        .flat_map(|g| [(g, None), (g, Some(0))])
        .collect()
}

fn options(grant: Option<usize>, spill: Option<usize>) -> OpenOptions {
    OpenOptions {
        create: Some(CreateOptions::default()),
        query_memory_bytes: grant.map(|g| g as u64),
        testing: Knobs::without_background_vacuum()
            .with_morsel_rows(Some(MORSEL))
            .with_spill_bytes(spill),
        ..OpenOptions::default()
    }
}

fn run(db: &Database<SimRuntime>, text: &str) -> Run {
    adtb_heap_cap::take_peak();
    let before = adtb_heap_cap::live();
    let result = db.query_text(text, &[], &adtb::CallOptions::default());
    let peak = adtb_heap_cap::take_peak().saturating_sub(before);
    let (charged, spilled) = db.testing().charged_peak();
    let ran = match result {
        Ok(mut rs) if rs.len() == 1 => Ran::Got(got_of(rs.remove(0))),
        Ok(rs) => Ran::Got(Got::Refused(format!("{} results of one query", rs.len()))),
        Err(Error::Rejected {
            reason:
                RejectReason::StatementLimit {
                    limit: Limit::Memory { bytes },
                },
        }) => Ran::Memory(bytes),
        Err(e) => Ran::Got(super::engine_failed(e)),
    };
    Run {
        ran,
        charged,
        uncharged: peak.saturating_sub(charged),
        spilled,
    }
}

/// What the grid reached.
#[derive(Default)]
struct Reached {
    /// The runs refused with no spill space that answered with it.
    spilled: usize,
    /// The runs that answered at a grant of the grid.
    answered: usize,
    /// The runs refused for their grant with spill space.
    refused: usize,
    /// The reads of the past that answered at a grant of the grid.
    past: usize,
}

/// The runs of one case: each text's run under the default grant and at each point.
struct Grid {
    texts: Vec<String>,
    want: Vec<Run>,
    at: Vec<(Point, Vec<Run>)>,
}

impl Grid {
    fn at(&self, p: Point) -> &[Run] {
        &self
            .at
            .iter()
            .find(|(q, _)| *q == p)
            .expect("a point of the grid")
            .1
    }
}

/// Returns the texts of `case`'s queries, each followed by its read at `position`.
fn both(case: &Case, position: &str) -> Vec<String> {
    case.queries
        .iter()
        .flat_map(texts)
        .flat_map(|t| {
            let past = format!("at commit {position} {{ {t} }}");
            [t, past]
        })
        .collect()
}

/// Returns the runs of `case` under the default grant and at every point of the grid.
fn grid(seed: u64, case: &Case) -> Grid {
    let log = vec![case.migration.clone()];
    process(
        &mut Sim::new(seed, Config::default()).without_trace(),
        |rt| {
            let open = |grant, spill| {
                Database::open_in(
                    rt.clone(),
                    "grid.adt",
                    &Log::new(log.clone()),
                    &options(grant, spill),
                )
                .unwrap_or_else(|e| panic!("the engine does not open: {e:?}"))
            };
            let db = open(None, None);
            write(
                &db,
                &["set setting RetentionWindow = { length: 1h }".to_owned()],
            );
            write(&db, &case.load);
            let mut txn = db
                .begin(&TxnOptions::default())
                .expect("a transaction begins");
            txn.query_text("count(x for x in [1])", &[], &TxnCallOptions::default())
                .expect("a read runs");
            let p = txn.commit(None).expect("a transaction commits");
            let texts = both(case, &format!("{{ epoch: {}, lsn: {} }}", p.epoch, p.lsn));
            let want: Vec<Run> = texts.iter().map(|t| run(&db, t)).collect();
            close(db);
            let mut at = Vec::new();
            for (g, spill) in points() {
                let db = open(Some(g), spill);
                let runs = texts.iter().map(|t| run(&db, t)).collect();
                at.push(((g, spill), runs));
                close(db);
            }
            Grid { texts, want, at }
        },
    )
}

/// Returns the first disagreement of `grid`.
fn disagreement(grid: &Grid, reached: &mut Reached) -> Option<String> {
    for (i, t) in grid.texts.iter().enumerate() {
        let past = i % 2 == 1;
        let present = i - usize::from(past);
        let w = match (&grid.want[present].ran, &grid.want[i].ran) {
            (Ran::Got(w), Ran::Got(got)) if !past || got == w => w,
            (Ran::Got(_), Ran::Got(Got::Refused(_))) if past => continue,
            (want, got) => {
                return Some(format!(
                    "`{t}` gives {got:?} under the default grant, its present read {want:?}"
                ));
            }
        };
        for ((g, spill), runs) in &grid.at {
            let r = &runs[i];
            match &r.ran {
                Ran::Got(got) if got == w => {
                    reached.answered += 1;
                    reached.past += usize::from(past);
                }
                Ran::Memory(bytes) if *bytes == *g as i64 => {
                    reached.refused += usize::from(spill.is_none());
                    if spill.is_none() && grid.want[i].charged <= *g {
                        return Some(format!(
                            "`{t}` is refused at a grant of {g} and spill {spill:?}, holding at \
                             most {} under the default grant",
                            grid.want[i].charged
                        ));
                    }
                }
                other => {
                    return Some(format!(
                        "`{t}` at a grant of {g} and spill {spill:?} gives {other:?}, {w:?} \
                         under the default grant"
                    ));
                }
            }
            let bound = grid.want[i].uncharged + SPILL_BUFFERS + DISK_COPIES * r.spilled;
            if r.uncharged > bound {
                return Some(format!(
                    "`{t}` at a grant of {g} and spill {spill:?} holds {} bytes outside it, past \
                     {bound}",
                    r.uncharged
                ));
            }
        }
        for g in grants() {
            let (on, off) = (&grid.at((g, None))[i].ran, &grid.at((g, Some(0)))[i].ran);
            if matches!(off, Ran::Got(_)) && matches!(on, Ran::Memory(_)) {
                return Some(format!(
                    "`{t}` at a grant of {g} answers with no spill space and is refused with it"
                ));
            }
            if matches!(off, Ran::Memory(_)) && matches!(on, Ran::Got(_)) {
                reached.spilled += 1;
            }
            if grants().contains(&(2 * g))
                && matches!(on, Ran::Got(_))
                && matches!(grid.at((2 * g, None))[i].ran, Ran::Memory(_))
            {
                return Some(format!(
                    "`{t}` answers at a grant of {g} and is refused at {}",
                    2 * g
                ));
            }
        }
    }
    None
}

/// Returns the first run of `big`, over twice the data of `small`, that holds more outside its
/// grant than the spill buffers and the disk's copies of its spill past what the same run holds
/// over `small`.
fn outgrown(small: &Grid, big: &Grid) -> Option<String> {
    for ((p, runs), (_, more)) in small.at.iter().zip(&big.at) {
        for (i, t) in small.texts.iter().enumerate() {
            let bound = runs[i].uncharged + SPILL_BUFFERS + DISK_COPIES * more[i].spilled;
            if more[i].uncharged > bound {
                return Some(format!(
                    "`{t}` at {p:?} holds {} bytes outside its grant over twice the data, past \
                     {bound}",
                    more[i].uncharged
                ));
            }
        }
    }
    None
}

#[test]
fn queries_under_a_grid_of_grants_answer_alike_or_are_refused_for_their_grant() {
    let mut reached = Reached::default();
    let mut found = Vec::new();
    let all = scripts();
    for seed in seeds() {
        let (small, big) = case_doubled(seed, QUERIES);
        let (small, big) = (grid(seed, &small), grid(seed, &big));
        let s = &all[Rng::new(seed).below(all.len() as u64) as usize];
        let corpus = grid(seed, &s.case(seed, QUERIES));
        let failed = [
            ("drawn", disagreement(&small, &mut reached)),
            ("drawn twice", disagreement(&big, &mut reached)),
            ("drawn twice", outgrown(&small, &big)),
            (s.name.as_str(), disagreement(&corpus, &mut reached)),
        ];
        for (label, m) in failed {
            if let Some(m) = m {
                found.push(format!("seed {seed}, {label}: {m}"));
            }
        }
    }
    assert_eq!(Vec::<String>::new(), found);
    profile::assert_reached("a run that spilled to answer", reached.spilled, 1);
    profile::assert_reached("a run that answered under the grid", reached.answered, 1);
    profile::assert_reached("a run refused for its grant", reached.refused, 1);
    profile::assert_reached("a read of the past that answered", reached.past, 1);
}
