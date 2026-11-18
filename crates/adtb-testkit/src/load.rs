//! Load rows into a database with statements, each row under versions no snapshot taken
//! afterwards sees, through any session that runs ad hoc statements.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::thread::LocalKey;

use adtb_lang::evolve::{Source, State};
use adtb_lang::schema::{Schema, Stage, ValueDef};
use adtb_lang::textual::{self, Form, Textual};
use adtb_lang::types::{RelId, Ty, TypeId};
use adtb_reference::Value;
use adtb_sim::{Config, Outcome, Sim, SimRuntime};

/// The transactions a load runs its statements in.
pub trait Session {
    type Txn;

    fn schema(&self) -> &Schema;
    fn begin(&self) -> Self::Txn;
    /// Begin a transaction that writes nothing and holds its snapshot until it ends.
    fn hold(&self) -> Self::Txn;
    /// Run ad hoc `text` in `tx` with `params`, each its name, type and value of that type.
    fn run(&self, tx: &mut Self::Txn, text: &str, params: &[(&str, Ty, Value)]);
    fn commit(&self, tx: Self::Txn);
    fn rollback(&self, tx: Self::Txn);
}

/// Whether a load leaves a transaction running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Running {
    Left,
    None,
}

/// Load `rows`, the rows of a reference database over `schema`, into the session by relation
/// name, holding a read-only transaction begun before the first write until the last ends.
/// With [`Running::Left`], begin afterwards the transaction [`running`] returns and return it.
///
/// Postconditions, for each row:
/// - the row is the one version of its key a snapshot begun afterwards sees;
/// - the file holds, hidden from such a snapshot, a version of the row's earlier value and one of
///   its deleted copy, each committed and deleted by a committed transaction;
/// - the file holds the row's aborted copy, inserted by a transaction that rolled back, unless a
///   write after it pruned that version from a full page;
/// - the earlier value, the deleted copy and the aborted copy each differ from the row and from
///   one another where a field outside the key is text or an instant; otherwise the copies are
///   the row and there is no earlier value;
/// - with [`Running::Left`], every committed noise version is present.
pub fn load<S: Session>(
    s: &S,
    schema: &Schema,
    rows: &adtb_reference::Database,
    running: Running,
) -> Option<S::Txn> {
    let sets = Sets::of(s.schema(), schema, rows);
    let held = s.hold();
    let once = |write: Write, version: Version| {
        let mut tx = s.begin();
        sets.run(s, &mut tx, write, version);
        s.commit(tx);
    };
    let mut aborted = s.begin();
    sets.run(s, &mut aborted, Write::Insert, Version::Aborted);
    s.rollback(aborted);
    once(Write::Insert, Version::Deleted);
    let mut tx = s.begin();
    s.run(&mut tx, &sets.deletes(), &[]);
    s.commit(tx);
    once(Write::Insert, Version::Earlier);
    once(Write::Replace, Version::Row);
    let left = (running == Running::Left).then(|| {
        let mut tx = s.begin();
        sets.run(s, &mut tx, Write::Replace, Version::Running);
        tx
    });
    s.rollback(held);
    left
}

/// Begin a transaction of the session, loaded with `rows` as [`load`] loads them, and in it
/// replace each row that has an earlier value by its running copy and delete each other row.
/// Return it, not ended. The running copy differs from the row and from its other copies as
/// [`load`] states.
///
/// Postconditions:
/// - its writes may have pruned committed noise versions from the full pages they went to;
///   [`load`] with [`Running::Left`] keeps them.
pub fn running<S: Session>(s: &S, schema: &Schema, rows: &adtb_reference::Database) -> S::Txn {
    let sets = Sets::of(s.schema(), schema, rows);
    let mut tx = s.begin();
    sets.run(s, &mut tx, Write::Replace, Version::Running);
    tx
}

#[derive(Clone, Copy)]
enum Write {
    Insert,
    Replace,
}

/// Which value of each row a statement writes.
#[derive(Clone, Copy)]
enum Version {
    Row,
    /// Changed by `~a` or 1 µs where it can be, else the row.
    Aborted,
    /// Changed by `~d` or 2 µs where it can be, else the row.
    Deleted,
    /// Changed by `~e` or 3 µs; only rows that can be changed.
    Earlier,
    /// Changed by `~r` or 4 µs; only rows that can be changed.
    Running,
}

impl Version {
    /// Returns the text suffix and instant offset of the version, `None` for the row itself.
    fn change(self) -> Option<(&'static str, i64)> {
        match self {
            Version::Row => None,
            Version::Aborted => Some(("~a", 1)),
            Version::Deleted => Some(("~d", 2)),
            Version::Earlier => Some(("~e", 3)),
            Version::Running => Some(("~r", 4)),
        }
    }

    /// Whether a row with no field to change still has this version, as the row itself.
    fn of_unchanged(self) -> bool {
        matches!(self, Version::Row | Version::Aborted | Version::Deleted)
    }
}

/// One relation's rows, as the session's schema holds them.
struct Relation {
    name: String,
    elem: Ty,
    from: (RelId, Vec<Value>),
}

struct Sets<'a> {
    from: &'a Schema,
    rels: Vec<Relation>,
}

impl<'a> Sets<'a> {
    /// Returns the rows of `rows`, a reference database over `from`, by the relation of `to` of
    /// the same module and name.
    fn of(to: &Schema, from: &'a Schema, rows: &adtb_reference::Database) -> Sets<'a> {
        let mut out: BTreeMap<RelId, Relation> = BTreeMap::new();
        for ((r, _), vs) in rows {
            let target = relation_of(to, from, *r);
            let rel = out.entry(target).or_insert_with(|| Relation {
                name: qualified(from, *r),
                elem: to.relation(target).elem.clone(),
                from: (*r, Vec::new()),
            });
            rel.from.1.extend(vs.iter().cloned());
        }
        Sets {
            from,
            rels: out.into_values().collect(),
        }
    }

    /// Returns `version` of every row of `rel` that has one.
    fn values(&self, rel: &Relation, version: Version) -> BTreeSet<Value> {
        let (r, rows) = &rel.from;
        rows.iter()
            .filter_map(|v| match version.change() {
                None => Some(v.clone()),
                Some(change) => match changed(self.from, *r, v, change) {
                    Some(c) => Some(c),
                    None => version.of_unchanged().then(|| v.clone()),
                },
            })
            .collect()
    }

    /// Returns the statement deleting every row of every relation.
    fn deletes(&self) -> String {
        let writes: Vec<String> = self
            .rels
            .iter()
            .map(|r| format!("delete x in {} where true", r.name))
            .collect();
        tuple(&writes)
    }

    /// Run in `tx` the statement inserting `version` of every row, replacing every row first
    /// when `write` is a replace.
    fn run<S: Session>(&self, s: &S, tx: &mut S::Txn, write: Write, version: Version) {
        let mut writes = Vec::new();
        let mut params = Vec::new();
        let names: Vec<String> = (0..self.rels.len()).map(|i| format!("rows{i}")).collect();
        for (r, name) in self.rels.iter().zip(&names) {
            if let Write::Replace = write {
                writes.push(format!("delete x in {} where true", r.name));
            }
            writes.push(format!("insert {} from ({name})", r.name));
            params.push((
                name.as_str(),
                Ty::Set(Box::new(r.elem.clone())),
                Value::Set(self.values(r, version)),
            ));
        }
        if !writes.is_empty() {
            s.run(tx, &tuple(&writes), &params);
        }
    }
}

/// Returns `rows`, a reference database over `from`, with each relation's rows under the
/// relation of `to` of the same module and name.
pub fn rows_of(
    to: &Schema,
    from: &Schema,
    rows: &adtb_reference::Database,
) -> adtb_reference::Database {
    rows.iter()
        .map(|((r, c), vs)| ((relation_of(to, from, *r), *c), vs.clone()))
        .collect()
}

fn qualified(schema: &Schema, rel: RelId) -> String {
    let def = schema.relation(rel);
    format!("{}::{}", schema.module(def.module).name, def.name)
}

/// Returns the relation of `to` of the module and name of relation `rel` of `from`.
pub fn relation_of(to: &Schema, from: &Schema, rel: RelId) -> RelId {
    let def = from.relation(rel);
    to.module_named(&from.module(def.module).name)
        .and_then(|m| match to.module(m).values.get(&def.name) {
            Some(&ValueDef::Relation(t)) => Some(t),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the database has no relation {}", qualified(from, rel)))
}

/// Returns `source` with each constructor of a module that is not the engine's that `from` stages
/// staged the same, found by the names of its relation or type and constructor and keyed by the
/// ids `source` gives them.
///
/// Preconditions: `source` builds.
pub fn staged_source(source: &Source, from: &Schema) -> Source {
    let mut out = source.clone();
    let to = source.schema().expect("the staged source builds");
    let state = |stage| State {
        stage,
        since: source.version,
    };
    let type_in = |module: &str, name: &str| {
        let m = to.module_named(module)?;
        to.module(m).types.get(name).copied()
    };
    for (r, def) in from.relations.iter().enumerate() {
        let module = from.module(def.module);
        if module.engine {
            continue;
        }
        let rel =
            to.module_named(&module.name)
                .and_then(|m| match to.module(m).values.get(&def.name) {
                    Some(&ValueDef::Relation(t)) => Some(t),
                    _ => None,
                });
        let Some(rel) = rel else { continue };
        for c in from.ty(def.elem_def).ctor_refs() {
            let stage = from.stage(RelId(r as u32), c.index());
            let name = &from.ty(c.def()).ctor_def(c).name;
            let ctor = to.ty(to.relation(rel).elem_def).ctor_named(name);
            if stage != Stage::Enabled
                && let (Some(rel), Some(ctor)) = (
                    to.ids().relation_id(rel),
                    ctor.and_then(|c| to.ids().ctor_id(c)),
                )
            {
                out.states.insert((rel, ctor), state(stage));
            }
        }
    }
    for (t, def) in from.types.iter().enumerate() {
        let module = from.module(def.module);
        if module.engine {
            continue;
        }
        let Some(to_def) = type_in(&module.name, &def.name) else {
            continue;
        };
        for (c, ctor) in def.ctors().iter().enumerate() {
            let stage = from.value_stage(TypeId(t as u32), c as u32);
            let id = to
                .ty(to_def)
                .ctor_named(&ctor.name)
                .and_then(|c| to.ids().ctor_id(c));
            if stage != Stage::Enabled
                && let Some(id) = id
            {
                out.value_states.insert(id, state(stage));
            }
        }
    }
    out
}

/// Returns the one statement of `writes`.
fn tuple(writes: &[String]) -> String {
    match writes {
        [one] => one.clone(),
        _ => format!("({})", writes.join(",\n")),
    }
}

/// Returns `row` of relation `rel` with `suffix` appended to each field outside the key that
/// is text and `micros` added to each that is an instant, or `None` when it has no such field.
fn changed(
    schema: &Schema,
    rel: RelId,
    row: &Value,
    (suffix, micros): (&str, i64),
) -> Option<Value> {
    let Value::Ctor(c, fs) = row else {
        panic!("a row is a constructor value")
    };
    let key = &schema.relation(rel).key;
    let mut any = false;
    let fs = fs
        .iter()
        .map(|(n, v)| {
            let w = match v {
                _ if key.contains(n) => v.clone(),
                Value::Text(t) => Value::Text(format!("{t}{suffix}")),
                Value::Instant(i) => Value::Instant(i + micros),
                other => other.clone(),
            };
            any |= &w != v;
            (n.clone(), w)
        })
        .collect();
    any.then_some(Value::Ctor(*c, fs))
}

/// Returns reference value `v` of type `ty` of `schema` as a value of `V`, through its textual
/// syntax.
pub fn convert<V: Textual>(schema: &Schema, ty: &Ty, v: &Value) -> V {
    let form = Form::of(schema, ty);
    let text = textual::print(v, &form).unwrap_or_else(|e| panic!("{v:?} prints: {e}"));
    textual::parse(&text, &form).unwrap_or_else(|e| panic!("`{text}` reads back: {e}"))
}

/// The contents of a simulator's files, by name.
pub type Files = BTreeMap<String, Vec<u8>>;

/// The files made for each declarations and rows.
pub type Made = RefCell<BTreeMap<(String, adtb_reference::Database), Files>>;

/// Returns the files `make` leaves in a simulator of seed 0, made once per thread in `cache` for
/// each declarations and rows.
///
/// Preconditions:
/// - every call with `cache`, `decls` and `rows` makes the same files.
pub fn made_once(
    cache: &'static LocalKey<Made>,
    decls: &str,
    rows: &adtb_reference::Database,
    make: impl FnOnce(&SimRuntime),
) -> Files {
    let key = (decls.to_owned(), rows.clone());
    if let Some(files) = cache.with_borrow(|m| m.get(&key).cloned()) {
        return files;
    }
    let mut sim = Sim::new(0, Config::default());
    let outcome = sim.run(|rt| make(&rt));
    assert_eq!(
        Outcome::Finished,
        outcome,
        "the files of `{decls}` are made"
    );
    let files = sim.files();
    cache.with_borrow_mut(|m| m.insert(key, files.clone()));
    files
}
