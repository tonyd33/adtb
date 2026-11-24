//! Run the cases of a corpus and report each directive whose outcome differs from its case.
//!
//! A case is one file of directives, named by its path under its corpus without `.test`.
//! Conformance files run on the reference interpreter and on the engine over the simulator, engine
//! scripts on the engine over the simulator. A directive that compares rows shows them as a diff
//! of the expected rows against the rows it got, and an update writes the rows it got in place of
//! the expected ones.

mod child;
pub mod cli;
mod diff;
mod update;

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;

use adtb_script::{Entry, case_name};

use crate::{Difference, Found, Got, Kind, Script, parse};

pub use adtb_script::shown;

pub use child::{decode, encode};

/// The directory of conformance files, relative to the workspace root.
pub const CONFORMANCE: &str = "conformance";
/// The directory of engine scripts, relative to the workspace root.
pub const SCRIPTS: &str = "crates/adtb-sim/scripts";

/// The directories of the scripts corpus that hold no case.
const NOT_AREAS: [&str; 2] = ["fixtures", "probes"];

/// The files at the root of the scripts corpus that are no script: the instructions of coding
/// agents.
const NOT_SCRIPTS: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

/// How the files of a corpus read and run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Conformance files, run on the reference and on the engine.
    Conformance,
    /// Engine scripts, run on the engine.
    Scripts,
}

/// A directory of case files of one format.
#[derive(Clone, Debug)]
pub struct Corpus {
    format: Format,
    root: PathBuf,
}

impl Corpus {
    pub fn new(format: Format, root: PathBuf) -> Corpus {
        Corpus { format, root }
    }
}

/// The sides a conformance case runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sides {
    Both,
    Reference,
    Engine,
}

impl Sides {
    fn reference(self) -> bool {
        self != Sides::Engine
    }

    fn engine(self) -> bool {
        self != Sides::Reference
    }
}

/// How a run takes its cases and what it does with a difference.
#[derive(Clone, Debug)]
pub struct Options {
    sides: Sides,
    /// Write the rows each differing directive got in place of its expected rows: the
    /// reference's for a conformance file, never the engine's.
    update: bool,
    /// Run only the cases whose name holds this text.
    include: Option<String>,
    /// How many cases run at once.
    jobs: usize,
    /// Stop at the first case that fails, running one case at a time.
    fail_fast: bool,
    color: bool,
    /// The program that runs one case on the engine in a process of its own, as `PROGRAM
    /// engine-case [--script] ROOT FILE`; `None` runs it in this process.
    isolate: Option<PathBuf>,
}

impl Options {
    /// Returns the options that run every case on `sides`, `jobs` at a time, in this process,
    /// without colour or update.
    pub fn new(sides: Sides, jobs: usize) -> Options {
        Options {
            sides,
            update: false,
            include: None,
            jobs,
            fail_fast: false,
            color: false,
            isolate: None,
        }
    }

    /// Returns whether `--include` selects `case`.
    fn includes(&self, case: &Case) -> bool {
        self.include
            .as_ref()
            .is_none_or(|include| case.name.contains(include.as_str()))
    }

    /// Returns the options that run each case on the engine in a process of `program`.
    pub fn isolated(self, program: PathBuf) -> Options {
        Options {
            isolate: Some(program),
            ..self
        }
    }
}

/// One case file of a corpus.
#[derive(Clone, Debug)]
pub struct Case {
    format: Format,
    root: PathBuf,
    path: PathBuf,
    /// The path under the root, without `.test`.
    name: String,
}

impl Case {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// The cases a selection names, and each entry it meets that is no case where its format
/// admits none.
#[derive(Debug, Default)]
pub struct Selection {
    cases: Vec<Case>,
    problems: Vec<Found>,
}

impl Selection {
    pub fn cases(&self) -> &[Case] {
        &self.cases
    }

    pub fn problems(&self) -> &[Found] {
        &self.problems
    }

    /// Returns the selection of every `parts`th case, in path order, from case `part`, with every
    /// problem.
    ///
    /// Preconditions:
    /// - `parts > 0`.
    pub fn part(self, part: usize, parts: usize) -> Selection {
        Selection {
            cases: self.cases.into_iter().skip(part).step_by(parts).collect(),
            problems: self.problems,
        }
    }
}

/// Returns the cases of `corpus` at `under`: the file `under` names, or each `.test` file below
/// the directory, sorted by path. In a conformance corpus any other file is passed over; in a
/// scripts corpus `fixtures`, `probes`, `AGENTS.md` and `CLAUDE.md` at its root are passed over
/// and any other file is a problem.
pub fn select(corpus: &Corpus, under: &Path) -> Selection {
    let mut out = Selection::default();
    if under.is_file() {
        out.cases.push(case(corpus, under.to_path_buf()));
        return out;
    }
    let mut files = Vec::new();
    walk(corpus, under, &mut files, &mut out.problems);
    files.sort();
    out.problems.sort_by(|a, b| a.head.cmp(&b.head));
    if files.is_empty() {
        out.problems.push(Found::at(
            None,
            format!("no cases under {}", shown(&corpus.root, under)),
        ));
    }
    out.cases = files.into_iter().map(|f| case(corpus, f)).collect();
    out
}

fn case(corpus: &Corpus, path: PathBuf) -> Case {
    Case {
        format: corpus.format,
        root: corpus.root.clone(),
        name: case_name(&corpus.root, &path),
        path,
    }
}

fn walk(corpus: &Corpus, dir: &Path, files: &mut Vec<PathBuf>, problems: &mut Vec<Found>) {
    let scripts = corpus.format == Format::Scripts;
    let at_root_in = |names: &[&str], path: &Path| {
        path.parent() == Some(corpus.root.as_path())
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| names.contains(&n))
    };
    let skip = |path: &Path| scripts && at_root_in(&NOT_AREAS, path);
    adtb_script::walk(dir, &skip, &mut |entry| match entry {
        Entry::Case(path) => files.push(path),
        Entry::Other(path) if scripts && !at_root_in(&NOT_SCRIPTS, &path) => {
            problems.push(Found::at(
                None,
                format!("{}: not a `.test` script", shown(&corpus.root, &path)),
            ));
        }
        Entry::Other(_) => {}
        Entry::Unread(dir, e) => problems.push(Found::at(
            None,
            format!("{}: {e}", shown(&corpus.root, &dir)),
        )),
    });
}

/// How one case ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Passed,
    /// Each directive that differs had its rows written.
    Updated,
    Failed(Vec<Found>),
    /// `--include` does not select the case.
    Skipped,
}

/// How a run ended: each case's name and verdict in the order of the cases, then each problem of
/// the selection.
#[derive(Debug, Default)]
pub struct Report {
    cases: Vec<(String, Verdict)>,
    problems: Vec<Found>,
    /// Whether `fail_fast` stopped the run before its last case.
    stopped: bool,
}

impl Report {
    fn count(&self, f: impl Fn(&Verdict) -> bool) -> usize {
        self.cases.iter().filter(|(_, v)| f(v)).count()
    }

    pub fn passed(&self) -> usize {
        self.count(|v| *v == Verdict::Passed)
    }

    pub fn updated(&self) -> usize {
        self.count(|v| *v == Verdict::Updated)
    }

    pub fn failed(&self) -> usize {
        self.count(|v| matches!(v, Verdict::Failed(_))) + self.problems.len()
    }

    pub fn skipped(&self) -> usize {
        self.count(|v| *v == Verdict::Skipped)
    }

    /// Returns whether no case failed and the selection held no problem.
    pub fn ok(&self) -> bool {
        self.failed() == 0
    }
}

/// Run every case of `selection` under `options`, writing a line for each case that fails or is
/// updated to `out` as it ends, then a summary and each failure with its differences.
pub fn run(
    selection: Selection,
    options: &Options,
    out: &mut (dyn Write + Send),
) -> io::Result<Report> {
    let Selection { cases, problems } = selection;
    let out = Mutex::new(out);
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let ended: Mutex<Vec<Option<Verdict>>> = Mutex::new(vec![None; cases.len()]);
    let jobs = if options.fail_fast {
        1
    } else {
        options.jobs.max(1)
    };
    let written: io::Result<()> = thread::scope(|s| {
        let workers: Vec<_> = (0..jobs)
            .map(|_| {
                s.spawn(|| -> io::Result<()> {
                    loop {
                        if stop.load(Ordering::Relaxed) {
                            return Ok(());
                        }
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(case) = cases.get(i) else {
                            return Ok(());
                        };
                        let verdict = verdict(case, options);
                        if options.fail_fast && matches!(verdict, Verdict::Failed(_)) {
                            stop.store(true, Ordering::Relaxed);
                        }
                        let line = diff::status(&case.name, &verdict, options.color);
                        if let Some(line) = line {
                            out.lock()
                                .expect("the output locks")
                                .write_all(line.as_bytes())?;
                        }
                        ended.lock().expect("the verdicts lock")[i] = Some(verdict);
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .try_for_each(|w| w.join().expect("a worker does not panic"))
    });
    written?;
    let ended = ended.into_inner().expect("the verdicts lock");
    let stopped = ended.iter().any(Option::is_none);
    let report = Report {
        cases: cases
            .iter()
            .zip(ended)
            .filter_map(|(c, v)| v.map(|v| (c.name.clone(), v)))
            .collect(),
        problems,
        stopped,
    };
    let out = out.into_inner().expect("the output locks");
    out.write_all(diff::summary(&report, options.color).as_bytes())?;
    Ok(report)
}

/// Run `case` and return its verdict, writing the rows of its differences when `options` asks
/// for an update.
fn verdict(case: &Case, options: &Options) -> Verdict {
    if !options.includes(case) {
        return Verdict::Skipped;
    }
    let found = differences(case, options);
    if found.is_empty() {
        return Verdict::Passed;
    }
    if options.update
        && let Some(rows) = writes(&found)
    {
        return match update::write(case, &rows) {
            Ok(()) => Verdict::Updated,
            Err(e) => Verdict::Failed(vec![Found::at(None, e)]),
        };
    }
    Verdict::Failed(found.into_iter().map(|(f, _)| f).collect())
}

/// Returns the rows of a directive and the line it starts on, when `f` compares rows.
fn written(f: &Found) -> Option<(usize, &Vec<String>)> {
    match (f.line, &f.rows) {
        (Some(line), Some((_, got))) => Some((line, got)),
        _ => None,
    }
}

/// Returns the rows an update writes for `found`, each difference marked with whether its rows
/// are written, by the line of its directive. Returns `None` when one difference has no rows to
/// write, or is not written and got rows other than those written.
fn writes(found: &[(Found, bool)]) -> Option<Vec<(usize, Vec<String>)>> {
    let written_rows: Vec<(usize, &Vec<String>)> = found
        .iter()
        .filter(|(_, w)| *w)
        .filter_map(|(f, _)| written(f))
        .collect();
    let covered = |f: &Found| written(f).is_some_and(|w| written_rows.contains(&w));
    found
        .iter()
        .filter(|(f, w)| *w || !covered(f))
        .map(|(f, w)| {
            let (line, got) = written(f).filter(|_| *w)?;
            Some((line, got.clone()))
        })
        .collect()
}

/// Returns each difference of `case`, with whether an update writes its rows.
fn differences(case: &Case, options: &Options) -> Vec<(Found, bool)> {
    let engine = || match &options.isolate {
        Some(program) => child::on_engine(program, case),
        None => on_engine(case),
    };
    match case.format {
        Format::Scripts => engine().into_iter().map(|f| (f, true)).collect(),
        Format::Conformance => {
            let mut out = Vec::new();
            if options.sides.reference() {
                out.extend(on_reference(case).into_iter().map(|f| (f, true)));
            }
            if options.sides.engine() {
                out.extend(engine().into_iter().map(|f| (f, false)));
            }
            out
        }
    }
}

/// Returns each difference of the conformance file `case` on the reference.
pub fn on_reference(case: &Case) -> Vec<Found> {
    conformance(case, "reference", crate::on_reference)
}

/// Returns each difference of `case` on the engine over the simulator, in this process.
pub fn on_engine(case: &Case) -> Vec<Found> {
    match case.format {
        Format::Conformance => conformance(case, "engine", crate::on_engine),
        #[cfg(feature = "scripts")]
        Format::Scripts => crate::scripted::run_file(&case.root, &case.path),
        #[cfg(not(feature = "scripts"))]
        Format::Scripts => {
            let at = shown(&case.root, &case.path);
            let message = format!("{at}: the runner is built without the `scripts` feature");
            vec![Found::at(None, message)]
        }
    }
}

/// Returns each difference `run` finds in the conformance file `case`, its head naming `side`.
fn conformance(
    case: &Case,
    side: &str,
    run: impl FnOnce(&Script, &Path) -> Vec<Difference>,
) -> Vec<Found> {
    let at = shown(&case.root, &case.path);
    let text = match fs::read_to_string(&case.path) {
        Ok(t) => t,
        Err(e) => return vec![Found::at(None, format!("{at}: {e}"))],
    };
    let script = match parse(&text) {
        Ok(s) => s,
        Err(d) => return vec![Found::at(None, format!("{at}:{}: {}", d.line, d.message))],
    };
    let dir = case.path.parent().unwrap_or(Path::new(""));
    run(&script, dir)
        .into_iter()
        .map(|d| {
            let expected = script.directives().iter().find_map(|x| match x.kind() {
                Kind::Query { rows, rowsort, .. } if x.line() == d.line => {
                    Some((rows.clone(), *rowsort))
                }
                _ => None,
            });
            let rows = match (expected, d.got) {
                (Some((mut expected, rowsort)), Some(Got::Rows(mut got))) => {
                    if rowsort {
                        expected.sort();
                        got.sort();
                    }
                    Some((expected, got))
                }
                _ => None,
            };
            Found {
                line: (d.line > 0).then_some(d.line),
                head: format!("{at}:{}: {side}: {}", d.line, d.message),
                values: None,
                rows,
            }
        })
        .collect()
}

/// Run every case of `corpus` at `under` without colour or update. Returns how many passed, or
/// what the run printed when one failed or the selection held no case.
pub fn passes(corpus: &Corpus, under: &Path, options: &Options) -> Result<usize, String> {
    passes_selected(select(corpus, under), options)
}

/// Run every case of `selection` as [`passes`] does.
pub fn passes_selected(selection: Selection, options: &Options) -> Result<usize, String> {
    let options = Options {
        update: false,
        color: false,
        ..options.clone()
    };
    let mut out = Vec::new();
    let report = run(selection, &options, &mut out).expect("a vector takes bytes");
    if report.ok() {
        Ok(report.passed())
    } else {
        Err(String::from_utf8_lossy(&out).into_owned())
    }
}
