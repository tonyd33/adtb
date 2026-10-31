//! The engine's background worker: one thread, holding no engine, that runs the background scrub.

use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use std::time::Duration;

use adtb_lang::schema::Schema;
use adtb_lang::types::RelId;
use adtb_sys::lock::Lock;

use crate::catalog::layout;
use crate::exec::{self, F64, Val};
use crate::runtime::{Instant, Io, Op, Runtime, ThreadId};
use crate::storage::{Pid, RetainedLog};
use crate::write::{Found, panic_text};

use super::command::engine_relation;
use super::micros;
use super::settings::Settings;
use super::verify::{Cursor, Job, Verifier};

/// The longest a pass runs between saves of its cursor.
const SAVE_EVERY: Duration = Duration::from_secs(60);

/// The longest the worker sleeps before it looks at the clock again.
const LONGEST_WAIT: Duration = Duration::from_secs(30 * 24 * 3600);

/// What the worker is told.
struct Control {
    every: Option<Duration>,
    io: F64,
    stop: bool,
    woken: bool,
    fail: Option<&'static str>,
    thread: Option<ThreadId>,
}

/// The engine's background worker.
pub(super) struct Background<R: Runtime> {
    rt: R,
    verifier: Arc<Verifier<R>>,
    schema: Arc<Schema>,
    findings: RelId,
    jobs: RelId,
    /// The wall instant a file with no cursor counts its first interval from, in microseconds.
    born: i64,
    control: Lock<Control>,
}

/// One pass over the file.
struct Pass<'b, R: Runtime> {
    log: RetainedLog,
    end: u64,
    job: Job<'b, R>,
    stamp: (i64, i64),
    found: BTreeSet<Found>,
    /// Whether an earlier process read the start of this pass, so that what it found stays.
    resumed: bool,
}

fn engine_table(schema: &Schema, name: &str) -> RelId {
    engine_relation(schema, "sys", name)
        .unwrap_or_else(|e| panic!("the engine declares `sys::{name}`: {e:?}"))
}

/// Returns the instant the first migration of the file was applied, if the file can say.
fn first_migration<R: Runtime>(verifier: &Verifier<R>, schema: &Schema) -> Option<i64> {
    let m = schema.module_named("catalog")?;
    let Some(adtb_lang::schema::ValueDef::Relation(rel)) =
        schema.module(m).values.get("migrations")
    else {
        return None;
    };
    let rows = verifier
        .catalog
        .committed_rows(&verifier.pager, verifier.db.txns(), *rel)
        .ok()?;
    rows.into_iter()
        .filter_map(|(c, row)| {
            let fields = layout::row(schema, *rel, c);
            fields.iter().zip(&row).find_map(|((name, t), v)| {
                let v = exec::from_stored(&exec::conv(schema, t), v);
                match v {
                    Val::Instant(at) if name.as_str() == "applied" => Some(at),
                    _ => None,
                }
            })
        })
        .min()
}

impl<R: Runtime> Background<R> {
    pub(super) fn new(
        verifier: Arc<Verifier<R>>,
        schema: Arc<Schema>,
        settings: &Settings,
    ) -> Arc<Background<R>> {
        let wall = micros(verifier.pager.wall());
        let born = if verifier.cursor().is_some() {
            wall
        } else {
            first_migration(&verifier, &schema).unwrap_or(wall)
        };
        Arc::new(Background {
            rt: verifier.rt.clone(),
            findings: engine_table(&schema, "verify_findings"),
            jobs: engine_table(&schema, "jobs"),
            verifier,
            schema,
            born,
            control: Lock::new(Control {
                every: settings.scrub,
                io: settings.io,
                stop: false,
                woken: false,
                fail: None,
                thread: None,
            }),
        })
    }

    /// Start the worker, unless the settings ask for no work.
    pub(super) fn start(self: &Arc<Self>) {
        let mut c = self.control.lock(&self.rt);
        self.spawn_if_wanted(&mut c);
    }

    /// Take the settings `settings`, with no reopen.
    pub(super) fn configure(self: &Arc<Self>, settings: &Settings) {
        let mut c = self.control.lock(&self.rt);
        c.every = settings.scrub;
        c.io = settings.io;
        c.woken = true;
        self.spawn_if_wanted(&mut c);
        if let Some(t) = c.thread {
            self.rt.unpark(t);
        }
    }

    fn spawn_if_wanted(self: &Arc<Self>, c: &mut Control) {
        if c.every.is_some() && c.thread.is_none() && !c.stop {
            let me = self.clone();
            c.thread = Some(self.rt.spawn_daemon("background", move || me.run()));
        }
    }

    /// Panic the worker's next read of a page with `text`.
    pub(super) fn fail(&self, text: &'static str) {
        let mut c = self.control.lock(&self.rt);
        c.fail = Some(text);
        c.woken = true;
        if let Some(t) = c.thread {
            self.rt.unpark(t);
        }
    }

    /// Stop the worker and wait for it to save where it is. No later call starts it again.
    pub(super) fn stop(&self) {
        let thread = {
            let mut c = self.control.lock(&self.rt);
            c.stop = true;
            c.thread.take()
        };
        if let Some(t) = thread {
            self.rt.unpark(t);
            self.rt.join(t);
        }
    }

    /// Run the worker. A panic in its read of a page ends it and leaves a `Structure` finding, the
    /// engine running. Any other panic crashes the process.
    fn run(&self) {
        let mut io = self.rt.io();
        let saved = self.verifier.cursor();
        let mut cursor = saved.unwrap_or(Cursor {
            next: None,
            started: self.born,
        });
        let mut saved = saved.unwrap_or(cursor);
        let mut pass: Option<Pass<'_, R>> = None;
        let mut save_at = self.rt.now() + SAVE_EVERY;
        loop {
            let (every, budget, stop) = {
                let c = self.control.lock(&self.rt);
                (c.every, c.io.get().min(1.0), c.stop)
            };
            if stop {
                break;
            }
            let Some(every) = every.filter(|_| budget > 0.0) else {
                pass = None;
                self.save(cursor, &mut saved);
                self.wait(&mut io, self.rt.now() + LONGEST_WAIT);
                continue;
            };
            let began = self.rt.now();
            if pass.is_none() {
                if let Some(wait) = self.begin_or_wait(&mut cursor, every) {
                    self.save(cursor, &mut saved);
                    let wait = wait.min(LONGEST_WAIT);
                    self.wait(&mut io, self.rt.now() + wait);
                    continue;
                }
                pass = Some(self.begin(cursor));
            }
            let current = pass.as_mut().expect("a pass began above");
            let page = cursor.next.expect("a running pass has a next page").0;
            let done = match self.read_page(current, page) {
                Ok(done) => done,
                Err(text) => {
                    current.found.insert(Found::Structure {
                        at: format!("the background worker stopped: {text}"),
                    });
                    self.verifier.keep_scrub(
                        &self.schema,
                        self.findings,
                        current.stamp,
                        &current.found,
                        true,
                    );
                    return;
                }
            };
            cursor.next = (!done).then_some(Pid(page + 1));
            if done {
                pass = None;
                self.save(cursor, &mut saved);
            } else if self.rt.now() >= save_at {
                self.save(cursor, &mut saved);
                save_at = self.rt.now() + SAVE_EVERY;
            }
            let gap = pace(self.rt.now().saturating_since(began), budget, every);
            if !gap.is_zero() {
                self.wait(&mut io, self.rt.now() + gap);
            }
        }
        drop(pass);
        self.save(cursor, &mut saved);
    }

    /// Returns how long until a pass is due, or moves `cursor` on to a pass due now.
    fn begin_or_wait(&self, cursor: &mut Cursor, every: Duration) -> Option<Duration> {
        if cursor.next.is_some() {
            return None;
        }
        let wall = micros(self.verifier.pager.wall());
        cursor.started = cursor.started.min(wall);
        let every = i64::try_from(every.as_micros()).unwrap_or(i64::MAX);
        let left = cursor.started.saturating_add(every).saturating_sub(wall);
        if left > 0 {
            return Some(Duration::from_micros(left.unsigned_abs()));
        }
        *cursor = Cursor {
            next: Some(Pid::FIRST_DATA),
            started: wall,
        };
        None
    }

    fn begin(&self, cursor: Cursor) -> Pass<'_, R> {
        let log = self.verifier.db.scrub_log();
        let end = self.verifier.pager.pages_within(&log);
        let (job, run, at) = self.verifier.scrub_row(self.jobs);
        let first = cursor.next.map_or(Pid::FIRST_DATA.0, |p| p.0);
        job.phase("scrub", done(first, end));
        Pass {
            log,
            end,
            job,
            stamp: (run, at),
            found: BTreeSet::new(),
            resumed: first > Pid::FIRST_DATA.0,
        }
    }

    /// Check page `page`, keep what it newly finds and, at the end of the file, end the pass.
    /// Returns whether the pass has ended, or the text of a panic in the read.
    fn read_page(&self, pass: &mut Pass<'_, R>, page: u64) -> Result<bool, String> {
        let v = &self.verifier;
        if page < pass.end {
            let fail = self.control.lock(&self.rt).fail.take();
            let read = caught(|| {
                if let Some(text) = fail {
                    panic!("{text}");
                }
                v.db.scrub_page(Pid(page), &pass.log)
            })?;
            let found: BTreeSet<Found> = read
                .into_iter()
                .filter(|f| !pass.found.contains(f))
                .collect();
            if !found.is_empty() {
                if v.keeps() {
                    let failed = v.db.quarantine_findings(&found);
                    pass.found.extend(failed);
                }
                pass.found.extend(found);
                v.keep_scrub(&self.schema, self.findings, pass.stamp, &pass.found, true);
            }
            if percent(page + 1, pass.end) != percent(page, pass.end) {
                pass.job.phase("scrub", done(page + 1, pass.end));
            }
        }
        let ended = page + 1 >= pass.end;
        if ended {
            v.keep_scrub(
                &self.schema,
                self.findings,
                pass.stamp,
                &pass.found,
                pass.resumed,
            );
        }
        Ok(ended)
    }

    fn save(&self, cursor: Cursor, saved: &mut Cursor) {
        if cursor != *saved
            && self
                .verifier
                .save_cursor(&self.schema, self.findings, cursor)
        {
            *saved = cursor;
        }
    }

    /// Sleep until `until`, or the settings change or the worker is stopped.
    fn wait(&self, io: &mut R::Io, until: Instant) {
        io.submit(Op::Timer { at: until });
        loop {
            while io.poll().is_some() {}
            {
                let mut c = self.control.lock(&self.rt);
                if c.stop || std::mem::take(&mut c.woken) {
                    return;
                }
            }
            if self.rt.now() >= until {
                return;
            }
            self.rt.park();
        }
    }
}

/// Returns what `body` returns, or the text of its panic. A fatal I/O error, and a payload with no
/// text, which is the runtime's own unwinding, go on up.
fn caught<T>(body: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(body))
        .map_err(|payload| panic_text(&*payload).unwrap_or_else(|| resume_unwind(payload)))
}

/// Returns how long to sleep after `spent` reading, so that reading takes `budget` of the time.
/// A budget too small to compute waits the interval, or a minute if that is longer, and rechecks.
fn pace(spent: Duration, budget: f64, every: Duration) -> Duration {
    Duration::try_from_secs_f64(spent.as_secs_f64() * (1.0 / budget - 1.0))
        .ok()
        .filter(|gap| *gap <= LONGEST_WAIT)
        .unwrap_or_else(|| every.clamp(SAVE_EVERY, LONGEST_WAIT))
}

fn done(page: u64, end: u64) -> f64 {
    if end == 0 {
        return 0.0;
    }
    (page as f64 / end as f64).min(1.0)
}

fn percent(page: u64, end: u64) -> u64 {
    page * 100 / end.max(1)
}
