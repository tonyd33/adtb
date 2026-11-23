//! Searches over generated migrations, applied by the checker as the engine records them: a
//! clause an operator rewrites covers the image of the rows it covered, a rename rewrites exactly
//! the tokens that resolve to its object, and the operators recorded as a migration applies
//! resolve as a replay of the log resolves them.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::panic::{self, AssertUnwindSafe};

use adtb::testing::chained_hash;
use adtb_lang::evolve::{ResolvedOp, Source};
use adtb_lang::ids::Space;
use adtb_lang::lex::{Kw, Tok, lex};
use adtb_lang::parse::{Options, parse_migration};
use adtb_lang::schema::{OnDelete, Schema, TypeDefKind};
use adtb_lang::span::FileId;
use adtb_lang::types::{RelId, TypeId};
use adtb_reference::Value;
use adtb_sim::{Config, Sim};
use adtb_testkit::corpus;
use adtb_testkit::fixture::Fixture;
use adtb_testkit::profile;
use adtb_testkit::schemas::{self, Kind, Migration, Rows};

use super::migrations::{held, opened, reference, stored_hash};
use super::{panic_message, seeds_of};

#[path = "migration_searches/coverage.rs"]
mod coverage;
#[path = "migration_searches/renames.rs"]
mod renames;

/// Cases per default run, times [`profile::DEEP`] under the profile `deep`, unless
/// `ADTB_DIFF_CASES` says otherwise.
const CASES: u64 = 1;

/// The statements that fill a case before its first migration and after each.
const STATEMENTS: usize = 64;

/// How many draws a scheduled migration gets before its kind is skipped.
const DRAWS: u64 = 4;

/// How many times its migrations a case may draw while one of its floors is unreached.
const STRETCH: u64 = 4;

/// Returns how many migrations are drawn over each case.
fn migrations() -> u64 {
    if profile::every_case() { 40 } else { 8 }
}

/// Returns how many migrations the replay search draws over each case: each replays the whole
/// log once more.
fn replayed() -> u64 {
    if profile::every_case() { 40 } else { 4 }
}

/// Returns the kind of migration `k` of the case of `seed`: under the profile `deep` every kind
/// of `kinds` in turn, by default spread across them.
fn scheduled(kinds: &[Kind], seed: u64, k: u64) -> Kind {
    let at = seed * migrations() + k;
    let n = kinds.len() as u64;
    let i = if profile::every_case() {
        at % n
    } else {
        at * 19 % n
    };
    kinds[i as usize]
}

/// A generated case, the migrations the checker applied to it as the engine records them, and
/// the source they leave.
struct Walk {
    seed: u64,
    case: schemas::Case,
    source: Source,
    log: Vec<String>,
    /// The operators each migration of `log` resolved to as it applied, in version order.
    resolved: Vec<Vec<ResolvedOp>>,
}

impl Walk {
    fn new(seed: u64) -> Result<Walk, String> {
        let case = schemas::case(seed, STATEMENTS);
        let first = corpus::migration(&case.decls())?;
        let mut walk = Walk {
            seed,
            case,
            source: Source::default(),
            log: Vec::new(),
            resolved: Vec::new(),
        };
        match walk.apply(&first)? {
            Some(_) => Ok(walk),
            None => Err("the checker refuses the schema".into()),
        }
    }

    /// Apply `text` as the engine records a migration it applies. Returns the schema it leaves,
    /// or `None` when the checker refuses it.
    fn apply(&mut self, text: &str) -> Result<Option<Schema>, String> {
        let file = parse_migration(text, Options::default())
            .ok()
            .map_err(|ds| format!("the migration does not parse: {ds:?}"))?;
        let Ok((next, compiled, recorded)) = self.source.record(&file, text, true) else {
            return Ok(None);
        };
        let schema = compiled.schema().clone();
        let ops = recorded
            .resolved()
            .map_err(|ds| format!("the migration does not resolve as it applies: {ds:?}"))?
            .to_vec();
        self.log.push(text.to_owned());
        self.resolved.push(ops);
        self.source = next;
        Ok(Some(schema))
    }

    /// Returns migration `k` of kind `kind` drawn over the walk's case, whole.
    fn draw(&self, k: u64, kind: Kind) -> Option<Migration> {
        self.draws(k, kind, DRAWS).next()
    }

    /// Returns the migrations `k` of kind `kind` drawn over the walk's case, whole, one for each
    /// of `n` seeds that draws one.
    fn draws(&self, k: u64, kind: Kind, n: u64) -> impl Iterator<Item = Migration> + '_ {
        let version = self.log.len() as u64 + 1;
        (0..n).filter_map(move |t| {
            self.case
                .migration(t << 48 | self.seed << 8 | k, kind, false, version)
        })
    }

    /// Returns `module app` and a `revert` of the last migration of the log.
    fn revert(&self) -> String {
        format!("module app\nrevert {:04}\n", self.log.len())
    }

    /// Returns why `folded`, the source a replay of the log folds to, is not the source the walk
    /// applied, or why a migration of the log from `from` on resolves on replay otherwise than
    /// it did as it applied.
    fn replays(&self, folded: &Source, from: u64) -> Option<String> {
        for (n, at) in (1..).zip(&self.resolved).skip(from as usize - 1) {
            match folded.resolved(n) {
                Ok(ops) if ops == *at => {}
                Ok(ops) => {
                    return Some(format!(
                        "migration {n} resolves on replay as\n{ops:?}\nand as it applied as\n{at:?}"
                    ));
                }
                Err(ds) => {
                    return Some(format!(
                        "migration {n} resolved as it applied and does not on replay: {ds:?}"
                    ));
                }
            }
        }
        let (a, f) = (&self.source, folded);
        let parts = [
            ("declarations", a.decls == f.decls),
            (
                "modules",
                (&a.modules, &a.installed) == (&f.modules, &f.installed),
            ),
            ("ids", a.ids == f.ids),
            (
                "states",
                (&a.states, &a.value_states) == (&f.states, &f.value_states),
            ),
            (
                "grants",
                format!("{:?}", a.grants) == format!("{:?}", f.grants),
            ),
            ("version", a.version == f.version),
        ];
        if let Some((what, _)) = parts.iter().find(|(_, same)| !same) {
            return Some(format!("the replayed source differs in its {what}"));
        }
        None
    }

    /// Returns why the hash the engine stored for a version of the log, the hash chained over
    /// the operators each migration resolved to as it applied, and the one chained over their
    /// resolution on replay `folded` differ, or why the last differs from the version hash.
    fn hashes(&self, folded: &Source) -> Option<String> {
        let schema = |s: &Source| s.check().map(|c| c.schema().clone()).ok();
        let (Some(applied), Some(replayed)) = (schema(&self.source), schema(folded)) else {
            return Some("the source does not check".into());
        };
        let mut sim = Sim::new(self.seed, Config::default());
        let role = opened(&mut sim, &self.log[..1], |db| {
            db.role_authority().create_role(schemas::ROLE)
        });
        if let Err(e) = role {
            return Some(format!("the engine does not open the schema: {e:?}"));
        }
        let stored = opened(&mut sim, &self.log, |db| {
            (1..=self.log.len())
                .map(|v| stored_hash(db, v))
                .collect::<Vec<_>>()
        });
        let stored = match stored {
            Ok(s) => s,
            Err(e) => return Some(format!("the engine does not open the log: {e:?}")),
        };
        let (mut a, mut r) = ([0; 32], [0; 32]);
        for ((n, at), stored) in (1..).zip(&self.resolved).zip(stored) {
            a = chained_hash(&applied, a, at);
            let ops = match folded.resolved(n) {
                Ok(ops) => ops,
                Err(ds) => return Some(format!("migration {n} does not resolve: {ds:?}")),
            };
            r = chained_hash(&replayed, r, &ops);
            let bytes = |h: [u8; 32]| Some(Value::Bytes(h.to_vec()));
            if stored != bytes(a) || a != r {
                return Some(format!(
                    "version {n} hashes as stored {stored:?}, as applied {a:?}, on replay {r:?}"
                ));
            }
        }
        match adtb::version_hash(folded) {
            Ok(h) if h == r => None,
            h => Some(format!(
                "the version hash is {h:?}, chained on replay {r:?}"
            )),
        }
    }
}

/// A broken invariant at migration `k` of the case of `seed`, with the log before it.
struct Failed {
    seed: u64,
    k: u64,
    log: Vec<String>,
    text: String,
    finding: String,
}

/// Run `body` on each seed of a run, catching a panic as a finding. Returns the failures.
fn each_seed(body: impl Fn(u64, &mut Option<Failed>) -> Option<Failed>) -> Vec<Failed> {
    let seeds: Vec<u64> = seeds_of(CASES).collect();
    let mut out = Vec::new();
    for seed in seeds {
        let mut at = None;
        let ran = panic::catch_unwind(AssertUnwindSafe(|| body(seed, &mut at)));
        match ran {
            Ok(f) => out.extend(f),
            Err(p) => {
                let mut f = at.unwrap_or(Failed {
                    seed,
                    k: 0,
                    log: Vec::new(),
                    text: String::new(),
                    finding: String::new(),
                });
                f.finding = format!("panics: {}", panic_message(&*p));
                out.push(f);
            }
        }
    }
    out
}

/// Panic with the failures of a search, when there are any.
fn report(search: &str, failures: &[Failed]) {
    let Some(first) = failures.first() else {
        return;
    };
    let mut out = format!(
        "{} seeds break {search}\nseed {}, migration {}: {}\nlog:\n{}\nmigration:\n{}\n",
        failures.len(),
        first.seed,
        first.k,
        first.finding,
        first.log.join("----\n"),
        first.text,
    );
    for f in failures {
        let line = f.finding.lines().next().unwrap_or_default();
        out.push_str(&format!("seed {}, migration {}: {line}\n", f.seed, f.k));
    }
    panic!("{out}");
}

/// The kinds the replay search draws: every kind but `script`, which the engine refuses; a
/// migration holding a `script` another kind draws is skipped.
fn replayed_kinds() -> Vec<Kind> {
    Kind::ALL
        .into_iter()
        .filter(|k| *k != Kind::Script)
        .collect()
}

#[test]
fn a_migration_resolves_as_it_applies_on_every_replay_of_the_log() {
    let kinds = replayed_kinds();
    let judged = std::sync::atomic::AtomicUsize::new(0);
    let reverts = std::sync::atomic::AtomicUsize::new(0);
    let hashed = std::sync::atomic::AtomicUsize::new(0);
    let failures = each_seed(|seed, at| {
        let mut walk = match Walk::new(seed) {
            Ok(w) => w,
            Err(e) => {
                return Some(Failed {
                    seed,
                    k: 0,
                    log: Vec::new(),
                    text: String::new(),
                    finding: e,
                });
            }
        };
        let mut folded = match adtb_lang::signature::replay(walk.log.iter().map(String::as_str)) {
            Ok(s) => s,
            Err(e) => {
                return Some(Failed {
                    seed,
                    k: 0,
                    log: walk.log.clone(),
                    text: String::new(),
                    finding: format!("the schema does not replay: {e:?}"),
                });
            }
        };
        let (mut mine, mut mine_reverted) = (0, 0);
        for k in 0..replayed() * STRETCH {
            if k >= replayed() && mine >= 4 && mine_reverted >= 1 {
                break;
            }
            let kind = scheduled(&kinds, seed, k);
            let Some(m) = walk.draw(k, kind) else {
                continue;
            };
            if m.ops().iter().any(|o| o.kind() == Kind::Script) {
                continue;
            }
            let mut texts = vec![(m.text(), false)];
            if m.exact() && k % 2 == 0 {
                texts.push((String::new(), true));
                if m.rereverted() && k % 4 == 0 {
                    texts.push((String::new(), true));
                }
            }
            let before = walk.case.clone();
            let mut forward = true;
            for (text, revert) in texts {
                let text = if revert { walk.revert() } else { text };
                *at = Some(Failed {
                    seed,
                    k,
                    log: walk.log.clone(),
                    text: text.clone(),
                    finding: String::new(),
                });
                let log = walk.log.clone();
                let failed = |finding: String| {
                    Some(Failed {
                        seed,
                        k,
                        log: log.clone(),
                        text: text.clone(),
                        finding,
                    })
                };
                match walk.apply(&text) {
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(e) => return failed(e),
                }
                let version = walk.log.len() as u64;
                folded = match adtb_lang::signature::replay_one(&folded, version, &text) {
                    Ok(s) => s,
                    Err(e) => return failed(format!("the log does not replay: {e:?}")),
                };
                if let Some(f) = walk.replays(&folded, version) {
                    return failed(f);
                }
                judged.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                mine += 1;
                if revert {
                    reverts.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    mine_reverted += 1;
                }
                walk.case = if forward {
                    m.applied(seed ^ k, 0)
                } else {
                    before.clone()
                };
                forward = !forward;
            }
        }
        let whole = walk
            .replays(&folded, 1)
            .or_else(|| walk.hashes(&folded))
            .map(|finding| Failed {
                seed,
                k: replayed() * STRETCH,
                log: walk.log.clone(),
                text: String::new(),
                finding,
            });
        if whole.is_none() {
            judged.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            hashed.fetch_add(walk.log.len(), std::sync::atomic::Ordering::Relaxed);
        }
        whole
    });
    report("apply and replay agreeing", &failures);
    let judged = judged.into_inner();
    profile::assert_reached("a migration judged", judged, 4);
    profile::assert_reached("a revert judged", reverts.into_inner(), 1);
    profile::assert_reached("a stored version hash judged", hashed.into_inner(), 4);
}

/// Assert each of `floors` was counted at least once in `counts`.
fn floors(counts: std::sync::Mutex<BTreeMap<&'static str, usize>>, floors: &[&'static str]) {
    let counts = counts
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let missed: Vec<&str> = floors
        .iter()
        .copied()
        .filter(|f| counts.get(f).copied().unwrap_or(0) == 0)
        .collect();
    assert_eq!(missed, Vec::<&str>::new(), "unreached: {counts:?}");
}

/// Returns the relations of module `app` of `schema`, by catalog id.
fn app_relations(schema: &Schema) -> Vec<(RelId, u64)> {
    let Some(app) = schema.module_named("app") else {
        return Vec::new();
    };
    schema
        .ids()
        .relation_ids()
        .filter(|(r, _)| schema.relation(*r).module == app)
        .map(|(r, id)| (r, u64::from(id)))
        .collect()
}

/// Returns the element type of relation `r`.
fn elem(schema: &Schema, r: RelId) -> TypeId {
    schema.relation(r).elem_def
}

/// Returns the name of the field `f` holds in `schema`, with its constructor's index.
fn field_name(schema: &Schema, f: adtb_lang::schema::FieldRef) -> (Option<u32>, String) {
    let t = schema.ty(f.def());
    let name = match f.ctor() {
        Some(c) => t.ctors()[c as usize].fields[f.pos() as usize].name.clone(),
        None => t.header()[f.pos() as usize].name.clone(),
    };
    (f.ctor(), name)
}

/// Returns `action` naming a retag's constructor by its catalog id in type `t` of `schema`.
fn action(schema: &Schema, t: TypeId, action: &OnDelete) -> String {
    match action {
        OnDelete::Restrict => "restrict".into(),
        OnDelete::Cascade => "cascade".into(),
        OnDelete::Clear => "clear".into(),
        OnDelete::Retag(c, _) => {
            let id = schema
                .ids()
                .type_ctor_ids(t)
                .find(|(r, _)| r.index() == *c)
                .map(|(_, id)| u64::from(id));
            format!("retag {id:?}")
        }
    }
}

/// Returns whether `t` is a sum written with constructors, not a record.
fn is_sum(schema: &Schema, t: TypeId) -> bool {
    matches!(schema.ty(t).kind, TypeDefKind::Sum { record: false, .. })
}

/// Returns the keys of `case`'s relation `rel` among `rows`, by row, with each row's constructor
/// and field names.
fn keyed(
    case: &schemas::Case,
    rows: &Rows,
    rel: &str,
) -> Vec<(Vec<Value>, String, BTreeSet<String>)> {
    let Some((_, key)) = case.relations().into_iter().find(|(r, _)| r == rel) else {
        return Vec::new();
    };
    rows.of(rel)
        .map(|rs| {
            rs.iter()
                .map(|(c, fs)| {
                    let k = key.iter().filter_map(|n| fs.get(n).cloned()).collect();
                    (k, c.clone(), fs.keys().cloned().collect())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Returns the ranges of the tokens of `text`, the end of text left out.
fn tokens(text: &str) -> Vec<(Tok, Range<usize>)> {
    lex(text)
        .0
        .into_iter()
        .filter(|t| t.tok != Tok::Eof)
        .map(|t| (t.tok, t.span.range()))
        .collect()
}

/// Returns declaration `i` of `s` as the file its names point into.
fn file_text(s: &Source, i: usize) -> String {
    s.file_text(FileId(i as u32))
}

/// Returns the reference holding `rows` of `case` after its statements' writes.
fn filled(case: &schemas::Case, rows: &Rows) -> Result<(Fixture, Rows), String> {
    let mut fx = reference(case, rows).map_err(|f| format!("{f:?}"))?;
    for s in case.stmts().iter().filter(|s| s.is_write()) {
        write(&mut fx, s.text())?;
    }
    let rows = held(&fx, case).map_err(|f| format!("{f:?}"))?;
    Ok((fx, rows))
}

/// Run write `text` on the reference `fx`; a refused write leaves its rows. Fail when the
/// reference panics.
fn write(fx: &mut Fixture, text: &str) -> Result<(), String> {
    match panic::catch_unwind(AssertUnwindSafe(|| fx.run(text))) {
        Ok(_) => Ok(()),
        Err(p) => Err(format!(
            "the reference panics on `{text}`: {}",
            panic_message(&*p)
        )),
    }
}
