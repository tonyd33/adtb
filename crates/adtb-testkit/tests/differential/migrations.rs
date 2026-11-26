//! Generated migrations over generated schemas, judged by invariants: a refusal names the
//! operator whose stated precondition fails, for the reason the precondition states; a commit
//! leaves the schema and the rows its operators state, as the generator's model applies them; a
//! clean `verify` and the store's page count after a sweep; the reference over the model's
//! schema agreeing with the engine; a revert restoring what an exact migration changed; and a
//! reopen replaying the log to the same catalog and version hash.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use adtb::testing::{Holder, Object};
use adtb::{Database, Error, OpenError, RejectReason};
use adtb_lang::ast::{BinOp, Expr, ExprKind, FnBody, Lit, UnOp};
use adtb_lang::evolve::Source;
use adtb_lang::ids::{ObjectKey, Space};
use adtb_lang::parse::{Options, parse_migration};
use adtb_lang::schema::{DefaultValue, OnDelete, Schema, Stage, TypeDefKind};
use adtb_lang::types::{ModuleId, RelId};
use adtb_reference::Value;
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_testkit::corpus;
use adtb_testkit::fixture::Fixture;
use adtb_testkit::schemas::{self, Kind, Migration, Reach, Rows};

use super::schemas::{Reached as Written, statements};
use super::{Got, close, engine_eval, panic_message, process, seeds_of, try_open};

#[path = "migrations/earlier.rs"]
mod earlier;
#[path = "migrations/reasons.rs"]
mod reasons;

/// Cases per default run, times [`adtb_testkit::profile::DEEP`] under the profile `deep`, unless
/// `ADTB_DIFF_CASES` says otherwise.
const CASES: u64 = 1;

/// Returns how many migrations are drawn over each case: enough under the profile `deep` for
/// [`scheduled`] to name every operator kind whole and broken more than once over its cases, in
/// the slots [`WANTED`] leaves it.
fn migrations() -> u64 {
    if adtb_testkit::profile::every_case() {
        44
    } else {
        3
    }
}

/// Returns how many migrations of each case [`scheduled`] names.
fn kinds_drawn() -> u64 {
    if adtb_testkit::profile::every_case() {
        migrations() - migrations() / WANTED_EVERY
    } else {
        migrations()
    }
}

/// The period of the slots of a case that take one of [`WANTED`] under the profile `deep`.
const WANTED_EVERY: u64 = 3;

/// The kind, brokenness and steered shape of the migrations the profile `deep` draws in turn
/// in every [`WANTED_EVERY`]th slot of a case, each kind or shape too rare otherwise to clear its
/// floor.
const WANTED: [(Kind, bool, Option<Reach>); 33] = [
    (Kind::Rekey, false, None),
    (Kind::AddRelation, false, Some(Reach::DroppedWaiting)),
    (Kind::Split, false, None),
    (Kind::Merge, false, None),
    (Kind::Merge, true, None),
    (Kind::RemoveConstructor, false, None),
    (Kind::Split, false, Some(Reach::RenamedCarry)),
    (Kind::Split, true, None),
    (Kind::Split, true, Some(Reach::UncarriedDefault)),
    (Kind::Split, false, Some(Reach::RenamedDefault)),
    (Kind::DropDefault, false, Some(Reach::DroppedBesideField)),
    (Kind::DropDefault, false, Some(Reach::RevertedRenamedFill)),
    (Kind::RenameField, false, Some(Reach::RenamedBesideDropped)),
    (Kind::MoveFieldDown, false, Some(Reach::MovedDownDefault)),
    (Kind::AddDefault, false, Some(Reach::DefaultOverSeveral)),
    (Kind::DropField, false, Some(Reach::DroppedBesideDefault)),
    (Kind::RenameField, false, Some(Reach::RenamedBesideDropped)),
    (Kind::MoveFieldDown, false, Some(Reach::MovedDownAction)),
    (
        Kind::AddConstructor,
        false,
        Some(Reach::ReplacedWhileUnbuilt),
    ),
    (Kind::Nest, true, Some(Reach::NestedDefault)),
    (Kind::DropDefault, false, Some(Reach::DroppedBesideField)),
    (Kind::AddDefault, false, Some(Reach::DefaultBeforeCtor)),
    (Kind::RenameField, true, Some(Reach::SharedElementRenamed)),
    (Kind::MoveFieldDown, false, Some(Reach::MovedDownAction)),
    (Kind::AddDefault, false, Some(Reach::DefaultOverSeveral)),
    (Kind::DropField, false, Some(Reach::DroppedBesideDefault)),
    (Kind::DropIndex, true, Some(Reach::DropAbsentClause)),
    (Kind::Deprecate, false, None),
    (Kind::RemoveConstructor, false, Some(Reach::RemovedAlone)),
    (Kind::Merge, false, Some(Reach::MergeUnderCheck)),
    (Kind::Unnest, false, None),
    (Kind::Deprecate, true, Some(Reach::DeprecateRetagTarget)),
    (Kind::Disable, true, Some(Reach::DisableRetagTarget)),
];

/// The cases that run at once, each worker thread taking the next seed no worker has taken. The
/// search's override under the profile `deep` holds as many test threads.
const AT_ONCE: usize = 8;

/// The statements that fill each case before its first migration, and that follow each
/// migration.
const STATEMENTS: usize = 24;

/// The sweeps that must settle the page count.
const SWEEPS: usize = 4;

/// How many draws a scheduled migration gets before its kind is skipped.
const DRAWS: u64 = 4;

/// The migrations a shrink of one finding may try.
const SHRINK_TRIES: usize = 24;

/// How often each operator kind was drawn, applied, and refused as the broken one, and each
/// shape applied.
#[derive(Debug, Default)]
struct Reached {
    drawn: BTreeMap<Kind, usize>,
    committed: BTreeMap<Kind, usize>,
    refused: BTreeMap<Kind, usize>,
    /// Each kind refused for a constructor it replaces that a stored tag names.
    tagged: BTreeMap<Kind, usize>,
    /// Each kind refused for a constructor it replaces that an object with no exact image under
    /// it names.
    unimaged: BTreeMap<Kind, usize>,
    /// Each shape committed, or refused as the broken operator.
    shapes: BTreeMap<Reach, usize>,
    /// Each kind of [`EARLIER`] a committed revert of an earlier migration undid, a `drop field`
    /// or `remove constructor` only when the defaults it was predicted to leave were not none.
    earlier: BTreeMap<Kind, usize>,
}

impl Reached {
    fn add(&mut self, other: Reached) {
        for (to, from) in [
            (&mut self.drawn, other.drawn),
            (&mut self.committed, other.committed),
            (&mut self.refused, other.refused),
            (&mut self.tagged, other.tagged),
            (&mut self.unimaged, other.unimaged),
            (&mut self.earlier, other.earlier),
        ] {
            for (k, n) in from {
                *to.entry(k).or_default() += n;
            }
        }
        for (r, n) in other.shapes {
            *self.shapes.entry(r).or_default() += n;
        }
    }

    /// Assert every operator kind was drawn, committed and refused as the broken one, and every
    /// shape committed, or refused as the broken operator.
    fn floors(&self) {
        let n = |m: &BTreeMap<Kind, usize>, kind| m.get(&kind).copied().unwrap_or(0);
        let missed: Vec<String> = Kind::ALL
            .into_iter()
            .flat_map(|kind| {
                [
                    ("drawn", n(&self.drawn, kind)),
                    ("committed", n(&self.committed, kind)),
                    ("refused", n(&self.refused, kind)),
                ]
                .into_iter()
                .filter(move |(what, n)| {
                    *n == 0 && (SCRIPT_COMMITS || (kind, *what) != (Kind::Script, "committed"))
                })
                .map(move |(what, _)| format!("{kind:?} {what}"))
            })
            .chain(
                Reach::ALL
                    .into_iter()
                    .filter(|r| !self.shapes.contains_key(r))
                    .map(|r| format!("{r:?} reached")),
            )
            .chain(
                TAGGED
                    .into_iter()
                    .filter(|k| !self.tagged.contains_key(k))
                    .map(|k| format!("{k:?} refused for a stored tag")),
            )
            .chain(
                UNIMAGED
                    .into_iter()
                    .filter(|k| !self.unimaged.contains_key(k))
                    .map(|k| format!("{k:?} refused for an object with no exact image")),
            )
            .chain(
                EARLIER
                    .into_iter()
                    .filter(|k| !self.earlier.contains_key(k))
                    .map(|k| format!("{k:?} undone by a revert after later migrations")),
            )
            .collect();
        assert_eq!(missed, Vec::<String>::new(), "kinds under their floor of 1");
    }
}

/// The kinds whose revert, after later migrations change fields or defaults, the search draws.
const EARLIER: [Kind; 3] = [Kind::DropDefault, Kind::DropField, Kind::RemoveConstructor];

/// The kinds besides those of [`EARLIER`] a migration a revert of it after later migrations
/// undoes may hold: none drops or carries a default or a field.
const ALONGSIDE: [Kind; 11] = [
    Kind::AddField,
    Kind::AddConstructor,
    Kind::AddType,
    Kind::AddModule,
    Kind::AddDefault,
    Kind::RenameType,
    Kind::RenameRelation,
    Kind::AddCheck,
    Kind::AddIndex,
    Kind::AddUnique,
    Kind::AddSet,
];

/// The most reverts of an earlier migration a case tries for each kind of [`EARLIER`], whether
/// a rename beside a dropped default picks the migration or the latest one holding the kind is
/// picked.
const EARLIER_TRIES: usize = 4;

/// The kinds of a migration after which the search reverts an earlier one of [`EARLIER`].
const EARLIER_AFTER: [Kind; 6] = [
    Kind::RenameField,
    Kind::AddField,
    Kind::DropField,
    Kind::AlterField,
    Kind::AddDefault,
    Kind::DropDefault,
];

/// The kinds that replace a constructor by no one constructor, refused when a stored tag names it.
const TAGGED: [Kind; 5] = [
    Kind::Split,
    Kind::Merge,
    Kind::Nest,
    Kind::Unnest,
    Kind::RemoveConstructor,
];

/// The kinds that refuse to replace a constructor an object with no exact image under them names.
const UNIMAGED: [Kind; 3] = [Kind::Split, Kind::Merge, Kind::Unnest];

/// Whether the engine applies a migration holding a `script`. It refuses every one.
const SCRIPT_COMMITS: bool = false;

/// A broken invariant: a signature a shrink keeps and what shows it.
#[derive(Clone, Debug)]
pub(super) struct Finding {
    sig: String,
    text: String,
}

fn found(sig: impl Into<String>, text: impl Into<String>) -> Finding {
    Finding {
        sig: sig.into(),
        text: text.into(),
    }
}

/// How a migration ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ended {
    Committed,
    Refused,
    /// Refused for a constructor an operator of this kind replaces that a stored tag names.
    Tagged(Kind),
}

/// A committed migration as a revert of it after later migrations reads it.
struct Undo {
    /// The kinds of its operators when a revert has what it needs to commit, else none.
    kinds: Vec<Kind>,
    /// The fill of each field it drops, by the field's path as it drops it.
    fills: Vec<(String, String)>,
    /// The catalog it left.
    left: earlier::Catalog,
    /// It drops a default, or a field whose default naming no constructor fills a field of
    /// another constructor, so a later rename may rename a field that default fills.
    renamed: bool,
}

impl Undo {
    /// Returns migration `m`, or the first migration when `None`, with the defaults of the
    /// catalog relations `cat` it leaves.
    fn of(m: Option<&Migration>, cat: &[Got]) -> Result<Undo, Finding> {
        let left = earlier::Catalog::of(cat).map_err(|e| found("the catalog does not read", e))?;
        let Some(m) = m.filter(|m| m.revertible()) else {
            return Ok(Undo {
                kinds: Vec::new(),
                fills: Vec::new(),
                left,
                renamed: false,
            });
        };
        let fills = m
            .ops()
            .iter()
            .filter(|o| o.kind() == Kind::DropField)
            .filter_map(|o| {
                let path = o.text().strip_prefix("drop field ")?;
                Some((path.to_owned(), o.fill()?.to_owned()))
            })
            .collect();
        Ok(Undo {
            kinds: m.ops().iter().map(schemas::Op::kind).collect(),
            fills,
            left,
            renamed: m.ops().iter().any(|o| {
                o.kind() == Kind::DropDefault || o.reach() == Some(Reach::DroppedBesideDefault)
            }),
        })
    }
}

/// A database a case's migrations run against, and what the judge knows of it.
struct Run {
    sim: Sim,
    seed: u64,
    log: Vec<String>,
    /// The source `log` replays to.
    source: Source,
    case: schemas::Case,
    /// The rows of `case`, as the reference holds them.
    rows: Rows,
    /// The catalog relations and stored version hash at the end of `log`.
    catalog: Vec<Got>,
    hash: Option<Value>,
    /// The pages allocated after a sweep in a session that opened `log` and wrote nothing.
    pages: usize,
    /// Each migration of `log` as a later revert of it reads it.
    undo: Vec<Undo>,
    /// Each kind a committed revert of an earlier exact migration holding it undid, as
    /// [`Reached`] counts it.
    earlier: BTreeMap<Kind, usize>,
    /// The reverts of an earlier migration tried for each kind it holds.
    tried: BTreeMap<Kind, usize>,
    /// The shapes committed reverts of an earlier migration reached.
    shapes: BTreeMap<Reach, usize>,
}

/// Returns the contents of each relation of `case` on `db`.
fn contents(db: &Database<SimRuntime>, case: &schemas::Case) -> Vec<Got> {
    case.relations()
        .iter()
        .map(|(r, _)| engine_eval(db, &format!("from x in {r} select x")))
        .collect()
}

/// Returns the rows of `case` on `db`.
fn rows(db: &Database<SimRuntime>, case: &schemas::Case) -> Result<Rows, Finding> {
    let mut values = Vec::new();
    for ((r, _), got) in case.relations().iter().zip(contents(db, case)) {
        match got {
            Got::Rows(vs) => values.push(vs),
            other => return Err(found("a relation does not read", format!("{r}: {other:?}"))),
        }
    }
    case.rows_of(&values)
        .map_err(|e| found("a row is not of the stated schema", e))
}

/// The catalog relations that hold the schema, its log and its grants.
const CATALOG: [&str; 19] = [
    "types",
    "constructors",
    "ctor_states",
    "value_states",
    "fields",
    "relations",
    "named_sets",
    "set_members",
    "indexes",
    "index_ctors",
    "constraints",
    "functions",
    "defaults",
    "computed",
    "migrations",
    "migration_ops",
    "migration_clauses",
    "roles",
    "grants",
];

fn catalog(db: &Database<SimRuntime>) -> Vec<Got> {
    CATALOG
        .iter()
        .map(|r| engine_eval(db, &format!("from x in catalog::{r} select x")))
        .collect()
}

/// Returns the names of the catalog relations `a` and `b` differ in.
fn catalog_diff(a: &[Got], b: &[Got]) -> Vec<&'static str> {
    CATALOG
        .iter()
        .zip(a.iter().zip(b))
        .filter(|(_, (x, y))| x != y)
        .map(|(r, _)| *r)
        .collect()
}

/// Returns the hash the migrations relation holds for version `version`.
pub(super) fn stored_hash(db: &Database<SimRuntime>, version: usize) -> Option<Value> {
    let q = format!(
        "from m in catalog::migrations where m.version == catalog::migrations.key({version}) select m.hash"
    );
    match engine_eval(db, &q) {
        Got::Rows(mut rows) if rows.len() == 1 => rows.pop(),
        _ => None,
    }
}

/// Returns the source `log` replays to, or why it does not.
fn replayed(log: &[String]) -> Result<Source, String> {
    adtb_lang::signature::replay(log.iter().map(String::as_str)).map_err(|e| format!("{e:?}"))
}

/// Returns the text of a literal, a name, a sum, a negation or an addition `e` writes, as
/// [`schemas::Case::shape`] writes it; `query` for a comprehension.
fn expr_text(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Lit(Lit::Int(n)) => n.clone(),
        ExprKind::Lit(Lit::Text(t)) => format!("{t:?}"),
        ExprKind::Lit(Lit::Bool(b)) => b.to_string(),
        ExprKind::Name(q) => q.name.text.clone(),
        ExprKind::Ctor { path, .. } => path.name.text.clone(),
        ExprKind::Unary(UnOp::Neg, x) => format!("-{}", expr_text(x)),
        ExprKind::Binary(BinOp::Add, a, b) => format!("{} + {}", expr_text(a), expr_text(b)),
        ExprKind::Comprehension(_) => "query".into(),
        other => format!("{other:?}"),
    }
}

/// Returns each line of the shape of module `app` on `db`, as [`schemas::Case::shape`] writes
/// it.
fn shape(db: &Database<SimRuntime>) -> Vec<String> {
    let schema = db.schema();
    let Some(app) = schema.module_named("app") else {
        return vec!["no module `app`".into()];
    };
    let mut out = relations_shape(schema, app);
    for t in schema.types.iter().filter(|t| t.module == app) {
        if let TypeDefKind::Sum { .. } = t.kind {
            let names: Vec<&str> = t.ctors().iter().map(|c| c.name.as_str()).collect();
            out.push(format!("type {} = {}", t.name, names.join(" | ")));
        }
    }
    for c in schema.checks.iter().filter(|c| c.module == app) {
        out.push(format!("check {}", c.name));
    }
    for c in schema.constraints.iter().filter(|c| c.module == app) {
        out.push(format!("constraint {}", c.name));
    }
    for f in schema.functions.iter().filter(|f| f.module == app) {
        let body = match &f.body {
            FnBody::Expr(e) => expr_text(e),
            other => format!("{other:?}"),
        };
        out.push(format!("fn {} = {body}", f.name));
        if f.stored.is_some() {
            out.push(format!("stored {}", f.name));
        }
    }
    for f in schema.aggregates.iter().filter(|f| f.module == app) {
        out.push(format!("aggregate {}", f.name));
    }
    for m in &schema.modules {
        if !m.engine && m.name != "app" {
            out.push(format!("module {}", m.name));
        }
    }
    let grants = engine_eval(
        db,
        &format!(
            "from g in catalog::grants where g.grantee == \"{}\" select {{ p: g.privilege, t: g.target }}",
            schemas::ROLE
        ),
    );
    match grants {
        Got::Rows(rows) => out.extend(rows.iter().map(grant_line)),
        other => out.push(format!("grants do not read: {other:?}")),
    }
    out.sort();
    out
}

/// Returns `grant P T` for a row `{ p, t }` of the grants relation, `T` the target up to its
/// first space.
fn grant_line(row: &Value) -> String {
    fn text(v: &Value) -> Option<&str> {
        match v {
            Value::Text(t) => Some(t),
            Value::Ctor(_, fs) | Value::Record(fs) => fs.iter().find_map(|(_, v)| text(v)),
            _ => None,
        }
    }
    let Value::Record(fs) = row else {
        return format!("grant {row:?}");
    };
    let field = |n: &str| fs.iter().find(|(f, _)| f == n).and_then(|(_, v)| text(v));
    let target = field("t").and_then(|t| t.split(' ').next()).unwrap_or("-");
    format!("grant {} {target}", field("p").unwrap_or("-"))
}

/// Returns the lines of the shape of the relations of module `app` of `schema`.
fn relations_shape(schema: &Schema, app: ModuleId) -> Vec<String> {
    let mut out = Vec::new();
    for (k, r) in schema.relations.iter().enumerate() {
        if r.module != app {
            continue;
        }
        let t = schema.ty(r.elem_def);
        out.push(format!(
            "relation {} : {} key ({})",
            r.name,
            t.name,
            r.key.join(", ")
        ));
        let ctors = t.ctors();
        let field = |f: &adtb_lang::schema::FieldDef| {
            let ty = schema.show_ty(&f.ty);
            let action = match &f.on_delete {
                _ if !ty.starts_with("Key ") && !ty.starts_with("Option (Key ") => String::new(),
                OnDelete::Restrict => " on delete restrict".into(),
                OnDelete::Cascade => " on delete cascade".into(),
                OnDelete::Clear => " on delete clear".into(),
                OnDelete::Retag(c, _) => format!(" on delete retag {}", ctors[*c as usize].name),
            };
            format!("{}: {ty}{action}", f.name)
        };
        let header: Vec<String> = t.header().iter().map(field).collect();
        let record = matches!(t.kind, TypeDefKind::Sum { record: true, .. });
        if !record {
            out.push(format!("header {}: {}", r.name, header.join(", ")));
        }
        for (c, ctor) in ctors.iter().enumerate() {
            let mut fs: Vec<String> = if record { header.clone() } else { Vec::new() };
            fs.extend(ctor.fields.iter().map(field));
            let stage = match schema.stage(RelId(k as u32), c as u32) {
                Stage::Enabled => "Enabled",
                Stage::Declared => "Declared",
                Stage::Deprecated => "Deprecated",
            };
            out.push(format!(
                "constructor {}.{} {stage}: {}",
                r.name,
                ctor.name,
                fs.join(", ")
            ));
        }
        let names = |cs: &Option<Vec<u32>>| match cs {
            Some(cs) if !record => {
                let mut ns: Vec<&str> = cs
                    .iter()
                    .map(|c| ctors[*c as usize].name.as_str())
                    .collect();
                ns.sort_unstable();
                format!(" in {}", ns.join(" | "))
            }
            _ => String::new(),
        };
        for x in &r.indexes {
            let verb = if x.unique { "unique" } else { "index" };
            out.push(format!(
                "{verb} {} ({}){}",
                r.name,
                x.text(0),
                names(&x.scope)
            ));
        }
        for s in &r.sets {
            out.push(format!(
                "set {} of {}{}",
                s.name,
                r.name,
                names(&Some(s.ctors.clone()))
            ));
        }
        for d in &r.defaults {
            let DefaultValue::Expr(e) = &d.value else {
                continue;
            };
            let at = match d.field.ctor() {
                Some(c) if !record => format!("{}.{}", r.name, ctors[c as usize].name),
                _ => r.name.clone(),
            };
            let name = &t.field_def(d.field).name;
            out.push(format!("default {at}.{name} = {}", expr_text(e)));
        }
        for c in &r.computed {
            out.push(format!(
                "computed {}.{}: {} = {}",
                r.name,
                c.name,
                schema.show_ty(&c.ty),
                expr_text(&c.expr)
            ));
        }
    }
    out
}

/// Returns, in line order, each copy of a line `a` holds more often than `b` marked `-`, and each
/// copy of one `b` holds more often marked `+`.
fn diff(a: &[String], b: &[String]) -> String {
    let count = |xs: &[String]| {
        let mut n: BTreeMap<String, i64> = BTreeMap::new();
        for x in xs {
            *n.entry(x.clone()).or_default() += 1;
        }
        n
    };
    let mut n = count(a);
    for (l, k) in count(b) {
        *n.entry(l).or_default() -= k;
    }
    let mut out: Vec<String> = Vec::new();
    for (l, k) in &n {
        let sign = if *k > 0 { '-' } else { '+' };
        out.extend((0..k.abs()).map(|_| format!("{sign}{l}")));
    }
    out.join("\n")
}

/// Returns the rows of `a` not in `b`, each marked `-`, then those of `b` not in `a`, `+`, by
/// relation of `case`.
fn rows_diff(case: &schemas::Case, a: &Rows, b: &Rows) -> String {
    let mut out = Vec::new();
    for (r, _) in case.relations() {
        let ra = a.of(&r).cloned().unwrap_or_default();
        let rb = b.of(&r).cloned().unwrap_or_default();
        out.extend(ra.difference(&rb).map(|x| format!("-{r} {x:?}")));
        out.extend(rb.difference(&ra).map(|x| format!("+{r} {x:?}")));
    }
    out.join("\n")
}

/// Returns `line` with the constructors of each parenthesized set `(A | B)` in name order.
fn sorted_scopes(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(open) = rest.find('(') {
        out.push_str(&rest[..=open]);
        rest = &rest[open + 1..];
        let inner = rest.find(')').map(|close| &rest[..close]);
        if let Some(inner) = inner
            && inner.contains(" | ")
            && !inner.contains('(')
        {
            let mut names: Vec<&str> = inner.split(" | ").collect();
            names.sort_unstable();
            out.push_str(&names.join(" | "));
            rest = &rest[inner.len()..];
        }
    }
    out.push_str(rest);
    out
}

/// Returns the declarations of `s` by module, white space collapsed, `on delete restrict` left
/// implicit, constructor sets and named sets' members in name order, the lines of a declaration
/// other than a type's sorted.
fn decls(s: &Source) -> BTreeSet<(String, String)> {
    s.decls
        .iter()
        .map(|d| {
            let mut lines: Vec<String> = d
                .text
                .lines()
                .map(|l| {
                    let mut l = sorted_scopes(&l.replace(" on delete restrict", ""));
                    if l.starts_with("set ")
                        && let Some((head, members)) = l.split_once(" = ")
                    {
                        let mut ms: Vec<&str> = members.split(" | ").map(str::trim).collect();
                        ms.sort_unstable();
                        l = format!("{head} = {}", ms.join(" | "));
                    }
                    l.split_whitespace().collect::<Vec<_>>().join(" ")
                })
                .filter(|l| !l.is_empty())
                .collect();
            if !lines.first().is_some_and(|l| l.starts_with("type ")) {
                lines.sort();
            }
            (d.module.clone(), lines.join(" "))
        })
        .collect()
}

/// Returns what of `was` the source `now`, after a migration and its revert, does not restore:
/// its declarations, modules and grants, and the id of each object the migration, which left
/// `mid`, kept.
fn unrestored(was: &Source, mid: &Source, now: &Source) -> Vec<String> {
    let mut out = Vec::new();
    let (dw, dn) = (decls(was), decls(now));
    for (m, t) in dw.difference(&dn) {
        out.push(format!("-[{m}] {t}"));
    }
    for (m, t) in dn.difference(&dw) {
        out.push(format!("+[{m}] {t}"));
    }
    if (&was.modules, &was.installed) != (&now.modules, &now.installed) {
        out.push(format!("modules {:?} -> {:?}", was.modules, now.modules));
    }
    let held: BTreeSet<(Space, u64)> = mid
        .ids()
        .iter()
        .map(|(k, id)| (k.kind().space(), *id))
        .collect();
    let kept = |k: &ObjectKey, id: u64| held.contains(&(k.kind().space(), id));
    for (k, id) in was.ids() {
        if let Some(n) = now.ids().get(k)
            && n != id
            && kept(k, *id)
        {
            out.push(format!("id of {k:?}: {id} -> {n}"));
        }
    }
    let all_kept = was.ids().iter().all(|(k, id)| kept(k, *id));
    if all_kept && was.grants != now.grants {
        out.push(format!("grants {:?} -> {:?}", was.grants, now.grants));
    }
    out
}

/// Returns the checker's refusal of `m` over `source`, if it refuses it.
fn judged(source: &Source, m: &Migration) -> Result<Option<adtb_lang::evolve::Refusal>, Finding> {
    let text = m.text();
    let file = parse_migration(&text, Options::default())
        .ok()
        .map_err(|ds| found("the migration does not parse", format!("{ds:?}")))?;
    match source.migrate(&file, &text) {
        Ok(_) => Ok(None),
        Err(adtb_lang::evolve::Failed::Refused(r)) => Ok(Some(r)),
        Err(adtb_lang::evolve::Failed::Broken(d)) => {
            Err(found("the migration model fails", d.message))
        }
    }
}

/// Returns whether the checker admits `m` over `source`, or why its judgement breaks the rule
/// that a migration is refused exactly when an operator breaks a stated precondition, for the
/// reason the precondition states.
fn checked(source: &Source, m: &Migration) -> Result<bool, Finding> {
    let refusal = judged(source, m)?;
    match (m.broken(), refusal) {
        (None, None) => Ok(true),
        (None, Some(r)) => {
            let why: Vec<&str> = r
                .operators
                .iter()
                .map(|d| d.message.as_str())
                .chain(r.recheck.iter().map(|d| d.diag.message.as_str()))
                .collect();
            Err(found(
                "the checker refuses operators meeting their preconditions",
                why.join("\n"),
            ))
        }
        (Some(b), None) => Err(found(
            format!("the checker admits a broken {:?}", m.ops()[b].kind()),
            m.ops()[b].text().to_owned(),
        )),
        (Some(b), Some(r)) => {
            let op = &m.ops()[b];
            let at = m.span(b);
            let own: Vec<&str> = r
                .operators
                .iter()
                .filter(|d| at.contains(&(d.span.start as usize)))
                .map(|d| d.message.as_str())
                .collect();
            let why: Vec<&str> = r
                .operators
                .iter()
                .map(|d| d.message.as_str())
                .chain(r.recheck.iter().map(|d| d.diag.message.as_str()))
                .collect();
            if let Err(want) = reasons::stated(op, &why, &own) {
                return Err(found(
                    format!(
                        "the checker refuses a broken {:?} for another reason",
                        op.kind()
                    ),
                    format!("{}\nwants {want}\ngives {why:?}", op.text()),
                ));
            }
            if m.ops().len() > 1
                && let Some(r) = judged(source, &m.without(b))?
            {
                return Err(found(
                    format!(
                        "the checker refuses the operators beside a broken {:?}",
                        op.kind()
                    ),
                    format!("{:?}\n{:?}", r.operators, r.recheck),
                ));
            }
            Ok(false)
        }
    }
}

/// Run `body` against the database at `log` in a process of `sim`. Returns its result, or the
/// open's failure.
pub(super) fn opened<T>(
    sim: &mut Sim,
    log: &[String],
    body: impl FnOnce(&Database<SimRuntime>) -> T,
) -> Result<T, OpenError> {
    process(sim, |rt| match try_open(rt, log) {
        Ok(db) => {
            let out = body(&db);
            close(db);
            Ok(out)
        }
        Err(e) => Err(e),
    })
}

/// Returns the number of pages the heaps and indexes of the relations of user modules on `db`
/// hold.
fn relation_pages(db: &Database<SimRuntime>) -> Result<usize, String> {
    let t = db.testing();
    let mut out = 0;
    for o in t.objects() {
        let user = matches!(
            t.holder(o),
            Ok(Holder::Relation {
                engine_module: false,
                ..
            })
        );
        let pages = match o {
            _ if !user => continue,
            Object::Index(_) | Object::HistoryIndex(_) => t.reachable(o).map(|(ps, _)| ps),
            _ => t.owned(o),
        };
        out += pages.map_err(|e| format!("{o:?}: {e:?}"))?.len();
    }
    Ok(out)
}

/// Vacuum every relation of `case` on `db` until a sweep leaves the pages of the relations of
/// user modules as many as the one before it left, at most [`SWEEPS`] times, checking after each
/// that `verify` finds nothing, no page leaked among them. Returns that count.
fn swept(db: &Database<SimRuntime>, case: &schemas::Case) -> Result<usize, Finding> {
    let sweep = || -> Result<usize, Finding> {
        let mut failed = Vec::new();
        for (r, _) in case.relations() {
            if let Err(e) = db.query_text(&format!("vacuum {r}"), &[], &Default::default()) {
                failed.push(format!("vacuum {r}: {e}"));
            }
        }
        let rows = db
            .testing()
            .verify_holding()
            .map(|rows| rows.map(|r| format!("{r:?}")).collect::<Vec<String>>())
            .map_err(|e| found("verify after a sweep fails", e.to_string()))?;
        if !rows.is_empty() || !failed.is_empty() {
            return Err(found(
                "verify after a sweep finds damage",
                format!("{failed:?} {rows:?}"),
            ));
        }
        relation_pages(db).map_err(|e| found("a relation's pages do not read", e))
    };
    let mut counts = vec![sweep()?];
    while counts.len() < SWEEPS {
        let next = sweep()?;
        if counts.last() == Some(&next) {
            return Ok(next);
        }
        counts.push(next);
    }
    Err(found(
        "sweeps do not settle the page count",
        format!("{counts:?}"),
    ))
}

/// Returns why a refusal of `m` by the engine, with `operator` and `detail`, is not the
/// refusal its broken operator's precondition states, or, with none broken, not one for data a
/// precondition reads.
fn engine_refusal(m: &Migration, operator: &str, detail: &str) -> Option<Finding> {
    let lines: Vec<&str> = detail.lines().collect();
    if let Some(b) = m.broken() {
        let op = &m.ops()[b];
        let named = op.text().lines().any(|l| l.trim() == operator);
        return match reasons::stated(op, &lines, &[]) {
            Ok(()) if named => None,
            Ok(()) => Some(found(
                format!(
                    "the engine names another operator refusing a broken {:?}",
                    op.kind()
                ),
                format!("`{operator}`: {detail}\n{}", op.text()),
            )),
            Err(want) => Some(found(
                format!(
                    "the engine refuses a broken {:?} for another reason",
                    op.kind()
                ),
                format!("`{operator}`: {detail}\nwants {want}\n{}", op.text()),
            )),
        };
    }
    let reading: Vec<&schemas::Op> = m.ops().iter().filter(|o| o.reads()).collect();
    let by = |o: &&schemas::Op, l: &str| reasons::data(o.kind(), l);
    let stated = lines.split_first().is_some_and(|(first, rest)| {
        reading.iter().any(|o| o.names(operator) && by(o, first))
            && rest.iter().all(|l| reading.iter().any(|o| by(o, l)))
    });
    (!stated).then(|| {
        found(
            "the engine refuses for data no operator's precondition reads",
            format!("`{operator}`: {detail}"),
        )
    })
}

/// Returns why a refusal of `op` of `m`, which replaces a constructor a stored tag names, with
/// `operator` and `detail`, does not name `op`, gives no line of `lines`, or gives a line neither
/// of `lines` nor a data refusal of an operator of `m` reading data.
fn tag_refusal(
    m: &Migration,
    op: &schemas::Op,
    lines: &BTreeSet<String>,
    operator: &str,
    detail: &str,
) -> Option<Finding> {
    let read = |l: &str| {
        m.ops()
            .iter()
            .any(|o| o.reads() && reasons::data(o.kind(), l))
    };
    let first = detail.lines().next().unwrap_or_default();
    let named = op.text().lines().any(|l| l.trim() == operator)
        || m.ops()
            .iter()
            .any(|o| o.reads() && o.names(operator) && reasons::data(o.kind(), first));
    let stated = detail.lines().any(|l| lines.contains(l))
        && detail.lines().all(|l| lines.contains(l) || read(l));
    (!named || !stated).then(|| {
        found(
            "the engine refuses an operator replacing a constructor a stored tag names otherwise",
            format!(
                "`{operator}`: {detail}\nwants one of {lines:?}\n{}",
                op.text()
            ),
        )
    })
}

impl Run {
    fn new(seed: u64) -> Result<Run, Finding> {
        let case = schemas::case(seed, STATEMENTS);
        let migration = corpus::migration(&case.decls())
            .map_err(|why| found("the checker refuses the schema", why))?;
        let log = vec![migration];
        let source = replayed(&log).map_err(|e| found("the first migration does not replay", e))?;
        let mut sim = Sim::new(seed, Config::default());
        let mut fx = reference(&case, &Rows::default())?;
        let ran = opened(&mut sim, &log, |db| {
            if let Err(e) = db.role_authority().create_role(schemas::ROLE) {
                panic!("the role is refused: {e}");
            }
            if let Some(f) = statements(&mut fx, db, &case, &mut Written::default()) {
                return Err(found(
                    format!("the reference disagrees before a migration: {}", f.sig),
                    f.text,
                ));
            }
            swept(db, &case)?;
            Ok((catalog(db), stored_hash(db, 1)))
        })
        .map_err(|e| found("the engine does not open the schema", format!("{e:?}")))??;
        let (catalog, hash) = ran;
        let undo = vec![Undo::of(None, &catalog)?];
        let rows = held(&fx, &case)?;
        Ok(Run {
            sim,
            seed,
            log,
            source,
            case,
            rows,
            catalog,
            hash,
            pages: 0,
            undo,
            earlier: BTreeMap::new(),
            tried: BTreeMap::new(),
            shapes: BTreeMap::new(),
        })
    }

    /// Returns the pages allocated after a sweep in a session that opens the log and writes
    /// nothing.
    fn settled(&mut self) -> Result<usize, Finding> {
        let case = &self.case;
        opened(&mut self.sim, &self.log, |db| swept(db, case))
            .map_err(|e| found("the database does not reopen", format!("{e:?}")))?
    }

    /// Delete the rows `m`'s operators need gone on the engine and the reference, judged
    /// alike, then sweep.
    fn empty(&mut self, m: &Migration) -> Result<(), Finding> {
        let emptied = m.emptied();
        if emptied.is_empty() {
            return Ok(());
        }
        let writes = self.case.emptying(&emptied);
        let mut fx = reference(&self.case, &self.rows)?;
        let log = self.log.clone();
        let case = &self.case;
        opened(&mut self.sim, &log, |db| {
            if let Some(f) = statements(&mut fx, db, &writes, &mut Written::default()) {
                return Err(found(
                    format!("the reference disagrees on a delete: {}", f.sig),
                    f.text,
                ));
            }
            swept(db, case).map(drop)
        })
        .map_err(|e| {
            found(
                "the database does not open to empty a relation",
                format!("{e:?}"),
            )
        })??;
        self.rows = held(&fx, &self.case)?;
        Ok(())
    }

    /// Apply `m` and judge it, advancing the run past it when it commits. Returns how it ended.
    fn step(&mut self, m: &Migration, k: u64) -> Result<Ended, Finding> {
        self.empty(m)?;
        self.pages = self.settled()?;
        let admitted = checked(&self.source, m)?;
        let text = m.text();
        let mut log = self.log.clone();
        log.push(text.clone());
        let version = log.len() as u64;
        let migrated = m.applied(self.seed.wrapping_mul(1009) ^ k, STATEMENTS);
        let applied = opened(&mut self.sim, &log, |db| {
            let shape = shape(db);
            let got = rows(db, &migrated);
            let sweep = swept(db, &migrated);
            (shape, got, sweep, catalog(db), stored_hash(db, log.len()))
        });
        let tagged = m
            .broken()
            .is_none()
            .then(|| m.tag_refusal(&self.rows))
            .flatten();
        let (shape, got, sweep, cat, hash) = match applied {
            Ok(_) if let Some((op, _)) = tagged => {
                return Err(found(
                    "the engine applies an operator replacing a constructor a stored tag names",
                    op.text().to_owned(),
                ));
            }
            Ok(x) => x,
            Err(OpenError::MigrationRefused { version: v, reason }) if v == version => {
                return self.refused(m, reason, tagged);
            }
            Err(e) => return Err(found("the migration fails, not refused", format!("{e:?}"))),
        };
        if !admitted {
            return Err(found(
                "the engine applies a migration the checker refuses",
                text,
            ));
        }
        let want = migrated.shape();
        if want != shape {
            return Err(found(
                "the schema after the migration is not the one its operators state",
                diff(&want, &shape),
            ));
        }
        let want_rows = m
            .rows(&self.rows)
            .map_err(|e| found("the model does not follow a committed operator", e))?;
        let got = got?;
        if got != want_rows {
            return Err(found(
                "the rows after the migration are not those its operators state",
                rows_diff(&migrated, &want_rows, &got),
            ));
        }
        sweep?;
        let source = self.replays(&log, &migrated, &cat, hash.as_ref(), &want_rows)?;
        if m.exact() {
            self.law(&log, &source, &migrated, &want_rows)?;
        }
        self.earlier_law(&log, m, &cat)?;
        let mut fx = reference(&migrated, &want_rows)?;
        let probes = migrated.probes();
        opened(&mut self.sim, &log, |db| {
            for case in [&migrated, &probes] {
                if let Some(f) = statements(&mut fx, db, case, &mut Written::default()) {
                    return Err(found(
                        format!("the reference disagrees after the migration: {}", f.sig),
                        f.text,
                    ));
                }
            }
            swept(db, &migrated).map(drop)
        })
        .map_err(|e| {
            found(
                "the database does not open after the migration",
                format!("{e:?}"),
            )
        })??;
        self.rows = held(&fx, &migrated)?;
        self.source = source;
        self.log = log;
        self.undo.push(Undo::of(Some(m), &cat)?);
        self.case = migrated;
        self.catalog = cat;
        self.hash = hash;
        Ok(Ended::Committed)
    }

    /// Check, on a copy of the store, when migration `m` that ends `log` changes a field or a
    /// default, a revert of the latest earlier migration of `log` whose operators are each of a
    /// kind of [`EARLIER`] or [`ALONGSIDE`], at most one `drop default`, holding a kind tried fewer than
    /// [`EARLIER_TRIES`] times, with a fill for each field it drops that the refusals ask for:
    /// that it commits leaving the defaults [`earlier::predict`] gives from `cat`, the catalog
    /// relations at the end of `log`, or is refused for a reason it predicts, and that a log it
    /// commits replays with every migration resolving. Counts the kind it undid when it commits,
    /// a `drop field` or `remove constructor` only when the defaults it predicts are not none.
    fn earlier_law(&mut self, log: &[String], m: &Migration, cat: &[Got]) -> Result<(), Finding> {
        let changes = m.ops().iter().any(|o| EARLIER_AFTER.contains(&o.kind()));
        let only = |u: &Undo| {
            u.kinds
                .iter()
                .all(|k| EARLIER.contains(k) || ALONGSIDE.contains(k))
                && u.kinds.iter().filter(|&&k| k == Kind::DropDefault).count() <= 1
        };
        let renames = m
            .ops()
            .iter()
            .any(|o| o.reach() == Some(Reach::RenamedBesideDropped));
        let open = |u: &Undo, k: &&Kind| {
            u.kinds.contains(k) && self.tried.get(k).copied().unwrap_or(0) < EARLIER_TRIES
        };
        let beside = self.undo.iter().enumerate().rev().find_map(|(j, u)| {
            let kind = EARLIER.iter().find(|k| open(u, k))?;
            (renames && u.renamed && only(u)).then_some((j, *kind))
        });
        let undone = beside.or_else(|| {
            self.undo.iter().enumerate().rev().find_map(|(j, u)| {
                EARLIER
                    .iter()
                    .find(|k| open(u, k))
                    .filter(|_| only(u))
                    .map(|&k| (j, k))
            })
        });
        let (Some((j, kind)), true) = (undone, changes) else {
            return Ok(());
        };
        *self.tried.entry(kind).or_default() += 1;
        let unread = |e: String| found("the catalog does not read", e);
        let now = earlier::Catalog::of(cat).map_err(unread)?;
        let fills = &self.undo[j].fills;
        let drops = self.undo[j].kinds.iter().filter(|&&k| k == Kind::DropField);
        let unfilled = fills.len() < drops.count();
        let predicted = earlier::predict(
            &now,
            &self.undo[j - 1].left,
            &self.undo[j].left,
            &log[j + 1..],
            unfilled,
        );
        let want = predicted.map_err(|e| found("the catalog does not read", e))?;
        let mut given: Vec<String> = Vec::new();
        let (reverted, after) = loop {
            let mut copy = Sim::with_files(self.seed, Config::default(), self.sim.files());
            let mut reverted = log.to_vec();
            let filled: String = given.iter().map(|f| format!("  fill {f}\n")).collect();
            reverted.push(format!("module app\nrevert {:04}\n{filled}", j + 1));
            let detail = match opened(&mut copy, &reverted, catalog) {
                Ok(after) => break (reverted, after),
                Err(OpenError::MigrationRefused {
                    version,
                    reason:
                        Error::Rejected {
                            reason: RejectReason::SchemaRefused { detail, .. },
                        },
                }) if version == reverted.len() as u64 => detail,
                Err(e) => {
                    return Err(found(
                        "the revert of an earlier migration fails, not refused",
                        format!("revert {:04}: {e:?}", j + 1),
                    ));
                }
            };
            let fill = earlier::unfilled(&detail).and_then(|(path, key)| {
                let (_, lit) = fills.iter().find(|(p, _)| p == path)?;
                let fill = format!("{key} = {lit}");
                (!given.contains(&fill)).then_some(fill)
            });
            if let Some(f) = fill {
                given.push(f);
                continue;
            }
            if reasons::reverted(&detail, &want.refusals) {
                return Ok(());
            }
            return Err(found(
                "the revert of an earlier migration is refused for no stated reason",
                format!("revert {:04}: {detail}", j + 1),
            ));
        };
        let Some(live) = &want.commits else {
            return Err(found(
                "the revert of an earlier migration commits where a stated reason refuses it",
                format!("revert {:04}: {:?}", j + 1, want.refusals),
            ));
        };
        let after = earlier::Catalog::of(&after).map_err(unread)?;
        let got = earlier::left(&now, &after).map_err(unread)?;
        if let Some(d) = earlier::differs(live, &got) {
            return Err(found(
                "the revert of an earlier migration leaves other defaults than it gives back",
                format!("revert {:04}: {d}", j + 1),
            ));
        }
        let shapes = earlier::renamed(&now, &self.undo[j - 1].left, &self.undo[j].left);
        let now = replayed(&reverted)
            .map_err(|e| found("the revert of an earlier migration does not replay", e))?;
        for n in 1..=now.version() {
            if let Err(ds) = now.resolved(n) {
                return Err(found(
                    "a migration of the log does not resolve after a revert of an earlier one",
                    format!("migration {n}: {ds:?}"),
                ));
            }
        }
        if kind == Kind::DropDefault || !live.is_empty() {
            *self.earlier.entry(kind).or_default() += 1;
        }
        for r in shapes {
            *self.shapes.entry(r).or_default() += 1;
        }
        Ok(())
    }

    /// Check that `log` replays to the version hash `hash` the engine stored, and that a reopen
    /// at it gives the catalog `cat` and the rows `want` of the relations of `case`. Returns the
    /// source the log replays to.
    fn replays(
        &mut self,
        log: &[String],
        case: &schemas::Case,
        cat: &[Got],
        hash: Option<&Value>,
        want: &Rows,
    ) -> Result<Source, Finding> {
        let source = replayed(log).map_err(|e| found("the log does not replay", e))?;
        let hashed = source
            .check()
            .map_err(|e| format!("{e:?}"))
            .and_then(|_| adtb::version_hash(&source).map_err(|e| format!("{e:?}")));
        let want_hash = hashed.map_err(|e| found("the log does not resolve", e))?;
        if hash != Some(&Value::Bytes(want_hash.to_vec())) {
            return Err(found(
                "the stored version hash is not the one the replayed log gives",
                format!("{hash:?} / {want_hash:?}"),
            ));
        }
        let reopened = opened(&mut self.sim, log, |db| {
            (catalog(db), stored_hash(db, log.len()), rows(db, case))
        })
        .map_err(|e| {
            found(
                "the database does not reopen after the migration",
                format!("{e:?}"),
            )
        })?;
        let same_rows = reopened.2.as_ref().is_ok_and(|r| r == want);
        if reopened.0 != cat || reopened.1.as_ref() != hash || !same_rows {
            return Err(found(
                "a reopen replays the log to another catalog or state",
                format!(
                    "catalog relations {:?}, hash {:?} / {hash:?}, rows equal {same_rows}",
                    catalog_diff(&reopened.0, cat),
                    reopened.1
                ),
            ));
        }
        Ok(source)
    }

    /// Judge a refusal of `m`: the reason the broken operator's precondition states, or for
    /// `tagged` the stored tag or a stated refusal of another operator of `m` reading data; then
    /// a reopen with `verify` clean, and the catalog, version hash, rows and swept page count as
    /// before it. Returns [`Ended::Tagged`] only for the stored tag's refusal.
    fn refused(
        &mut self,
        m: &Migration,
        reason: Error,
        tagged: Option<(&schemas::Op, BTreeSet<String>)>,
    ) -> Result<Ended, Finding> {
        let Error::Rejected {
            reason: RejectReason::SchemaRefused { operator, detail },
        } = &reason
        else {
            return Err(found(
                "the migration is refused for an unstated reason",
                format!("{reason:?}"),
            ));
        };
        let mut ended = Ended::Refused;
        let wrong = match tagged {
            Some((op, lines)) => match tag_refusal(m, op, &lines, operator, detail) {
                None => {
                    ended = Ended::Tagged(op.kind());
                    None
                }
                Some(f) => engine_refusal(m, operator, detail).map(|_| f),
            },
            None => engine_refusal(m, operator, detail),
        };
        if let Some(f) = wrong {
            return Err(f);
        }
        let log = self.log.clone();
        let case = &self.case;
        let now = opened(&mut self.sim, &log, |db| {
            (
                catalog(db),
                stored_hash(db, log.len()),
                rows(db, case),
                swept(db, case),
            )
        })
        .map_err(|e| {
            found(
                "the database does not reopen after a refusal",
                format!("{e:?}"),
            )
        })?;
        let (cat, hash, got, pages) = now;
        if cat != self.catalog || hash != self.hash {
            return Err(found(
                "a refused migration changes the catalog",
                format!(
                    "catalog relations {:?}, hash {hash:?} / {:?}",
                    catalog_diff(&cat, &self.catalog),
                    self.hash
                ),
            ));
        }
        let got = got?;
        if got != self.rows {
            return Err(found(
                "a refused migration changes the data",
                rows_diff(&self.case, &self.rows, &got),
            ));
        }
        let pages = pages?;
        if pages != self.pages {
            return Err(found(
                "a refused migration changes the page count a sweep leaves",
                format!("{} before, {pages} after", self.pages),
            ));
        }
        Ok(ended)
    }

    /// Check, on a copy of the store, that a revert of the migration `log` ends with restores
    /// the source and the rows before it, and that a revert of that revert gives
    /// the schema of `migrated` and the rows `after` the migration left.
    fn law(
        &mut self,
        log: &[String],
        mid: &Source,
        migrated: &schemas::Case,
        after: &Rows,
    ) -> Result<(), Finding> {
        let mut copy = Sim::with_files(self.seed, Config::default(), self.sim.files());
        let mut reverted = log.to_vec();
        reverted.push(format!("module app\nrevert {:04}\n", log.len()));
        let before = &self.case;
        let restored = opened(&mut copy, &reverted, |db| rows(db, before)).map_err(|e| {
            found(
                "the revert of an exact migration is refused",
                format!("{e:?}"),
            )
        })??;
        if restored != self.rows {
            return Err(found(
                "the revert of an exact migration does not restore the data",
                rows_diff(before, &self.rows, &restored),
            ));
        }
        let now = replayed(&reverted).map_err(|e| found("the revert does not replay", e))?;
        let lost = unrestored(&self.source, mid, &now);
        if !lost.is_empty() {
            return Err(found(
                "the revert of an exact migration does not restore the schema",
                lost.join("\n"),
            ));
        }
        for n in 1..=now.version() {
            if let Err(ds) = now.resolved(n) {
                return Err(found(
                    "a migration of the log does not resolve after a revert",
                    format!("migration {n}: {ds:?}"),
                ));
            }
        }
        reverted.push(format!("module app\nrevert {:04}\n", reverted.len()));
        let again =
            opened(&mut copy, &reverted, |db| (shape(db), rows(db, migrated))).map_err(|e| {
                found(
                    "the revert of the revert of an exact migration is refused",
                    format!("{e:?}"),
                )
            })?;
        let want = migrated.shape();
        if again.0 != want || again.1.as_ref().ok() != Some(after) {
            return Err(found(
                "the revert of a revert does not apply the migration again",
                diff(&want, &again.0),
            ));
        }
        Ok(())
    }
}

/// Returns the reference over the schema `case`'s model declares, its constructors in the
/// model's write states, holding `rows`.
pub(super) fn reference(case: &schemas::Case, rows: &Rows) -> Result<Fixture, Finding> {
    let decls = case.decls();
    let mut fx = panic::catch_unwind(AssertUnwindSafe(|| Fixture::new(&decls)))
        .map_err(|p| found("the model's schema does not check", panic_message(&*p)))?;
    for (r, c, stage) in case.closed() {
        let rel = fx.rel(&r);
        fx.stage_relation((rel, c), stage);
    }
    let values = case
        .values(rows)
        .map_err(|e| found("the model's rows are not of its schema", e))?;
    for ((r, _), held) in case.relations().iter().zip(values) {
        let rel = fx.rel(r);
        for (c, v) in held {
            fx.db.entry((rel, c)).or_default().insert(v);
        }
    }
    Ok(fx)
}

/// Returns the rows the reference `fx` holds of the relations of `case`.
pub(super) fn held(fx: &Fixture, case: &schemas::Case) -> Result<Rows, Finding> {
    let values: Vec<Vec<Value>> = case
        .relations()
        .iter()
        .map(|(r, _)| {
            let rel = fx.rel(r);
            fx.db
                .iter()
                .filter(|((s, _), _)| *s == rel)
                .flat_map(|(_, rows)| rows.iter().cloned())
                .collect()
        })
        .collect();
    case.rows_of(&values)
        .map_err(|e| found("the reference holds rows of another schema", e))
}

/// Returns the kind, brokenness and steered shape of migration `k` of the case of `seed`: under
/// the profile `deep` every [`WANTED_EVERY`]th one of [`WANTED`] in turn, the others as [`scheduled`] names
/// them.
fn slot(seed: u64, k: u64) -> (Kind, bool, Option<Reach>) {
    if adtb_testkit::profile::every_case() && k % WANTED_EVERY == WANTED_EVERY - 1 {
        let at = seed * (migrations() / WANTED_EVERY) + k / WANTED_EVERY;
        return WANTED[at as usize % WANTED.len()];
    }
    let (kind, broke) = scheduled(seed, k - k / WANTED_EVERY);
    (kind, broke, None)
}

/// Returns the kind and brokenness of migration `k` of the case of `seed`: under the profile
/// `deep` kinds in turn, each whole and then broken, a `remove` after the `deprecate` it needs;
/// by default spread across the kinds.
fn scheduled(seed: u64, k: u64) -> (Kind, bool) {
    let at = seed * kinds_drawn() + k;
    let n = Kind::ALL.len() as u64;
    let kind = if adtb_testkit::profile::every_case() {
        at / 2 % n
    } else {
        at * 19 % n
    };
    (Kind::ALL[kind as usize], at % 2 == 1)
}

/// What one case found: the migration that broke an invariant, its position and the log
/// before it, and the finding.
struct Failed {
    k: u64,
    log: Vec<String>,
    migration: Option<Migration>,
    finding: Finding,
}

/// Run the migrations of the case of `seed`, counting what they reach, up to migration `last`
/// in place of which `instead` runs when given. Returns the first broken invariant.
fn run(seed: u64, reached: &mut Reached, last: u64, instead: Option<&Migration>) -> Option<Failed> {
    let mut at = (0, Vec::new(), None);
    let ran = panic::catch_unwind(AssertUnwindSafe(|| {
        let mut run = match Run::new(seed) {
            Ok(r) => r,
            Err(finding) => return Some((Vec::new(), None, finding)),
        };
        let mut pending = Vec::new();
        for k in 0..=last {
            pending.push(slot(seed, k));
            let version = run.log.len() as u64 + 1;
            let drawn = pending
                .iter()
                .enumerate()
                .find_map(|(p, &(kind, broke, want))| {
                    (0..DRAWS)
                        .find_map(|t| {
                            run.case.migration(
                                t << 48 | seed << 8 | k,
                                (kind, broke, want),
                                version,
                            )
                        })
                        .map(|m| (p, kind, broke, m))
                });
            let Some((p, kind, broke, m)) = drawn else {
                continue;
            };
            pending.remove(p);
            let m = match instead {
                Some(m) if k == last => m.clone(),
                _ => m,
            };
            at = (k, run.log.clone(), Some(m.clone()));
            let first = m.prefix(1).expect("a migration holds its first step");
            if !first
                .ops()
                .iter()
                .any(|o| o.kind() == kind && o.broken() == broke)
            {
                let f = found(
                    "the migration's first step is not of its scheduled kind",
                    format!("{kind:?}, broken {broke}"),
                );
                return Some((run.log.clone(), Some(m), f));
            }
            match run.step(&m, k) {
                Ok(ended) => {
                    if let Ended::Tagged(kind) = ended {
                        *reached.tagged.entry(kind).or_default() += 1;
                    }
                    for o in m.ops() {
                        *reached.drawn.entry(o.kind()).or_default() += 1;
                        let to = match (ended, o.broken()) {
                            (Ended::Committed, _) => &mut reached.committed,
                            (Ended::Refused | Ended::Tagged(_), true) => &mut reached.refused,
                            (Ended::Refused | Ended::Tagged(_), false) => continue,
                        };
                        *to.entry(o.kind()).or_default() += 1;
                        if ended != Ended::Committed && o.unimaged() {
                            *reached.unimaged.entry(o.kind()).or_default() += 1;
                        }
                        let judged =
                            |r: Reach| !r.transient() || ended == Ended::Committed && m.exact();
                        if let (Ended::Committed | Ended::Refused, Some(r)) = (ended, o.reach())
                            && judged(r)
                        {
                            *reached.shapes.entry(r).or_default() += 1;
                        }
                    }
                }
                Err(finding) => return Some((run.log.clone(), Some(m), finding)),
            }
        }
        for (k, n) in &run.earlier {
            *reached.earlier.entry(*k).or_default() += n;
        }
        for (r, n) in &run.shapes {
            *reached.shapes.entry(*r).or_default() += n;
        }
        None
    }));
    let (k, log, migration) = at;
    match ran {
        Ok(None) => None,
        Ok(Some((log, migration, finding))) => Some(Failed {
            k,
            log,
            migration,
            finding,
        }),
        Err(p) => {
            let message = panic_message(&*p);
            Some(Failed {
                k,
                log,
                migration,
                finding: found(
                    format!("panics: {}", message.lines().next().unwrap_or_default()),
                    message,
                ),
            })
        }
    }
}

/// Returns `failed` with its migration shrunk to its shortest prefix of drawn steps, each with
/// the model of the schema it leaves, that shows a finding of the same signature.
fn shrink(seed: u64, failed: Failed) -> Failed {
    let Some(m) = failed.migration.clone() else {
        return failed;
    };
    for n in (1..m.drawn_steps()).take(SHRINK_TRIES) {
        let smaller = m.prefix(n).expect("a prefix of the migration's steps");
        if let Some(f) = run(seed, &mut Reached::default(), failed.k, Some(&smaller))
            && f.finding.sig == failed.finding.sig
        {
            return f;
        }
    }
    failed
}

/// Run the case of each of `seeds` on `workers` threads, each taking the next seed no thread has
/// taken. Returns each seed with what its case reached and its first broken invariant, by seed.
fn run_all(seeds: &[u64], workers: usize, last: u64) -> Vec<(u64, Reached, Option<Failed>)> {
    let next = AtomicUsize::new(0);
    let mut ran: Vec<_> = std::thread::scope(|scope| {
        let pool: Vec<_> = (0..workers.min(seeds.len()))
            .map(|_| {
                scope.spawn(|| {
                    let mut ran = Vec::new();
                    while let Some(&seed) = seeds.get(next.fetch_add(1, Relaxed)) {
                        let mut reached = Reached::default();
                        let failed = run(seed, &mut reached, last, None);
                        ran.push((seed, reached, failed));
                    }
                    ran
                })
            })
            .collect();
        pool.into_iter()
            .flat_map(|w| w.join().expect("a case reports its panic as a finding"))
            .collect()
    });
    ran.sort_by_key(|&(seed, ..)| seed);
    ran
}

#[test]
fn generated_migrations_keep_their_invariants() {
    if adtb_testkit::profile::every_case() {
        // About twice the measured peak of eight seeds at once, 1.56 GB.
        adtb_heap_cap::raise_cap(3 << 30);
    }
    let seeds = seeds_of(CASES);
    let last = migrations() - 1;
    let all: Vec<u64> = seeds.clone().collect();
    let ran = run_all(&all, AT_ONCE, last);
    let mut reached = Reached::default();
    let mut failures = Vec::new();
    for (seed, r, failed) in ran {
        reached.add(r);
        failures.extend(failed.map(|f| (seed, f)));
    }
    let profile = if adtb_testkit::profile::every_case() {
        "deep"
    } else {
        "default"
    };
    if adtb_testkit::profile::every_case() {
        eprintln!(
            "shapes reached: {:?}\nkinds undone by an earlier revert: {:?}",
            reached.shapes, reached.earlier
        );
    }
    if let Some((seed, first)) = failures.first() {
        let small = shrink(
            *seed,
            Failed {
                k: first.k,
                log: first.log.clone(),
                migration: first.migration.clone(),
                finding: first.finding.clone(),
            },
        );
        let text = |f: &Failed| {
            f.migration
                .as_ref()
                .map(Migration::text)
                .unwrap_or_default()
        };
        let mut report = format!(
            "{} of {} seeds break an invariant under the profile {profile}\n\
             seed {seed}, migration {}: {}: {}\nlog:\n{}\nmigration:\n{}\ndrawn as:\n{}\n",
            failures.len(),
            seeds.end - seeds.start,
            small.k,
            small.finding.sig,
            small.finding.text,
            small.log.join("----\n"),
            text(&small),
            text(first),
        );
        for (seed, f) in &failures {
            report.push_str(&format!(
                "seed {seed}, migration {}: {}: {}\n",
                f.k, f.finding.sig, f.finding.text
            ));
        }
        panic!("{report}");
    }
    adtb_testkit::profile::assert_reached(
        "a source's build memo hit checked against an uncached build",
        adtb_lang::sampled::BUILDS.checked() as usize,
        1,
    );
    adtb_testkit::profile::assert_reached(
        "a model's build memo hit checked against an uncached build",
        adtb_lang::sampled::MODEL_BUILDS.checked() as usize,
        1,
    );
    adtb_testkit::profile::assert_reached(
        "a use of the shared engine ids checked against ids computed again",
        adtb_lang::sampled::ENGINE_IDS.checked() as usize,
        1,
    );
    adtb_testkit::profile::assert_reached(
        "a held check of a build checked against its schema checked again",
        adtb_lang::sampled::CHECKS.checked() as usize,
        1,
    );
    adtb_testkit::profile::assert_reached(
        "a held replay of a logged migration checked against the migration applied again",
        adtb_lang::sampled::REPLAYS.checked() as usize,
        1,
    );
    if adtb_testkit::profile::every_case() {
        reached.floors();
    } else {
        let all = |m: &BTreeMap<Kind, usize>| m.values().sum::<usize>();
        adtb_testkit::profile::assert_reached("a commit", all(&reached.committed), 1);
        adtb_testkit::profile::assert_reached("a refusal", all(&reached.refused), 1);
    }
}
