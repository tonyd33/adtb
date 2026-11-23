//! The permission analysis against evaluation: what the analysis leaves uncharged changes no
//! outcome, every change a write makes is charged, and a grant set admits a statement exactly
//! when it covers what the statement reads and writes.

use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::check::{Adhoc, check_adhoc};
use adtb_lang::ids::CatFieldId;
use adtb_lang::parse::{Options, parse_adhoc};
use adtb_lang::permission::{
    EffectiveGrant, EffectiveGrants, Mode, Requirement, Requirements, Right, Target,
};
use adtb_lang::schema::CtorRef;
use adtb_lang::types::RelId;
use adtb_reference::{Abort, Database, Interpreter, Outcome, Rejection, Value};
use adtb_testkit::fixture::Fixture;
use adtb_testkit::fuzz::{SCHEMA, adhoc_text};
use adtb_testkit::profile;

const ROWS: &str = r#"transact
  let a = insert items { name: "a", n: 1, status: Draft }
  let b = insert items { name: "b", n: 2, status: Sent { at: 5 } }
  let c = insert items { name: "c", n: 3, status: Paid { at: 7, by: "p" } }
  let d = insert items { name: "d", n: 4, status: Paid { at: 9, by: "q" } }
  insert tags.Small { item: key a, size: 3 }
  insert tags.Small { item: key b, size: 8 }
  insert tags.Large { item: key c, label: "l" }
  insert tags.Large { item: key d, label: "m" }
  return 0"#;

/// The most steps a fixpoint takes before the run aborts; a drawn bound can ask a million.
const FIX_STEPS: usize = 64;

/// A stored object the analysis can charge a read of.
#[derive(Clone, Debug)]
enum Read {
    Ctor(RelId, u32),
    Field(RelId, u32, String),
}

impl Read {
    fn rel(&self) -> RelId {
        match self {
            Read::Ctor(rel, _) | Read::Field(rel, ..) => *rel,
        }
    }
}

/// A stored row and the index of its constructor.
type Row = (u32, Value);

struct World {
    f: Fixture,
}

impl World {
    fn new(schema: &str, rows: &str) -> World {
        let mut f = Fixture::new(schema);
        f.ok(rows);
        World { f }
    }

    fn target(&self, rel: RelId, c: u32) -> Target {
        let s = self.f.schema();
        let td = s.ty(s.relation(rel).elem_def);
        Target {
            relation: s.ids().relation_id(rel).expect("a relation id"),
            constructor: s
                .ids()
                .ctor_id(td.ctor_at(c).expect("a constructor"))
                .expect("a constructor id"),
        }
    }

    fn field(&self, rel: RelId, c: u32, name: &str) -> (Target, CatFieldId) {
        let s = self.f.schema();
        let td = s.ty(s.relation(rel).elem_def);
        let field = td
            .field(td.ctor_at(c), name)
            .expect("a field of the constructor");
        (
            self.target(rel, c),
            s.ids().field_id(field).expect("a field id"),
        )
    }

    /// Returns grant set `n` of the 125 over `ts`, with the requirements it covers: for each of
    /// `A`, `B` and `C`, a base-5 digit of `n` grants no read, a read of no field, of `x`, of
    /// `y`, or of both; every other right on every relation is granted.
    fn grant_set(&self, n: usize) -> (EffectiveGrants, BTreeSet<Requirement>) {
        let mut rights = Vec::new();
        let mut covered = BTreeSet::new();
        for read in self.reads() {
            match read {
                Read::Ctor(rel, c) => {
                    let t = self.target(rel, c);
                    covered.extend([Requirement::Insert(t), Requirement::Delete(t)]);
                    rights.extend([Right::Insert(t), Right::Delete(t)]);
                    for to in 0..self
                        .f
                        .schema()
                        .ty(self.f.schema().relation(rel).elem_def)
                        .ctors()
                        .len() as u32
                    {
                        let (from, to) = (t.constructor, self.target(rel, to).constructor);
                        rights.push(Right::Retag {
                            relation: t.relation,
                            from,
                            to,
                        });
                        covered.insert(Requirement::Retag {
                            relation: t.relation,
                            from,
                            to,
                        });
                    }
                    let names: Vec<String> = self
                        .reads()
                        .into_iter()
                        .filter_map(|r| match r {
                            Read::Field(r, k, name) if (r, k) == (rel, c) => Some(name),
                            _ => None,
                        })
                        .collect();
                    let fields: BTreeSet<CatFieldId> =
                        names.iter().map(|f| self.field(rel, c, f).1).collect();
                    rights.push(Right::Update {
                        target: t,
                        fields: fields.clone(),
                    });
                    covered.extend(fields.iter().map(|&f| Requirement::UpdateField(t, f)));
                    let read: Option<Vec<&str>> = if rel == self.f.rel("ts") {
                        match n / 5usize.pow(c) % 5 {
                            0 => None,
                            1 => Some(vec![]),
                            2 => Some(vec!["x"]),
                            3 => Some(vec!["y"]),
                            _ => Some(vec!["x", "y"]),
                        }
                    } else {
                        Some(names.iter().map(String::as_str).collect())
                    };
                    if let Some(read) = read {
                        let fields: BTreeSet<CatFieldId> =
                            read.iter().map(|f| self.field(rel, c, f).1).collect();
                        covered.insert(Requirement::ReadConstructor(t));
                        covered.extend(fields.iter().map(|&f| Requirement::ReadField(t, f)));
                        rights.push(Right::Read { target: t, fields });
                    }
                }
                Read::Field(..) => {}
            }
        }
        let relations = rights
            .into_iter()
            .map(|right| EffectiveGrant {
                right,
                grant_option: false,
            })
            .collect();
        let grants = EffectiveGrants {
            relations,
            read_files: false,
        };
        (grants, covered)
    }

    /// Returns whether `r` reads a key field, which comes with any read of its constructor.
    fn is_key_read(&self, r: &Requirement) -> bool {
        let Requirement::ReadField(t, field) = r else {
            return false;
        };
        let s = self.f.schema();
        let f = s.ids().field_of(*field).expect("a field of the schema");
        let rel = s
            .ids()
            .relation_of(t.relation)
            .expect("a relation of the schema");
        s.relation(rel)
            .key
            .contains(&s.ty(f.def()).field_def(f).name)
    }

    fn requirement(&self, read: &Read) -> Requirement {
        match read {
            Read::Ctor(rel, c) => Requirement::ReadConstructor(self.target(*rel, *c)),
            Read::Field(rel, c, name) => {
                let (t, field) = self.field(*rel, *c, name);
                Requirement::ReadField(t, field)
            }
        }
    }

    /// Returns each constructor of each relation of the module, and each of its fields but the
    /// key.
    fn reads(&self) -> Vec<Read> {
        let s = self.f.schema();
        let mut out = Vec::new();
        for (i, r) in s.relations.iter().enumerate() {
            if s.module(r.module).engine {
                continue;
            }
            let rel = RelId(i as u32);
            let td = s.ty(r.elem_def);
            for c in 0..td.ctors().len() as u32 {
                out.push(Read::Ctor(rel, c));
                let ctor = td.ctor_at(c);
                for (f, def) in td.field_refs() {
                    if f.ctor()
                        .is_none_or(|own| Some(own) == ctor.map(CtorRef::index))
                        && !r.key.contains(&def.name)
                    {
                        out.push(Read::Field(rel, c, def.name.clone()));
                    }
                }
            }
        }
        out
    }

    fn run(&self, adhoc: &Adhoc, db: &Database) -> Result<Outcome, Abort> {
        let schema = adhoc.schema(self.f.schema());
        Interpreter::new(schema, self.f.program(), db)
            .with_fix_limit(FIX_STEPS)
            .run(
                &adhoc.terms,
                adhoc.items[0],
                &[],
                &mut self.f.supply.clone(),
            )
    }

    /// Returns the writes the change of `out` makes, as the requirements each needs.
    fn written(&self, out: &Outcome) -> BTreeSet<Requirement> {
        let s = self.f.schema();
        let interp = self.f.interp();
        let mut by_key: BTreeMap<(RelId, Value), [Option<Row>; 2]> = BTreeMap::new();
        for (side, rows) in [&out.change.deleted, &out.change.inserted]
            .into_iter()
            .enumerate()
        {
            for (&(rel, c), set) in rows {
                if rel.0 as usize >= s.relations.len() || s.temp_id(rel).is_some() {
                    continue;
                }
                for row in set {
                    by_key.entry((rel, interp.key_of(rel, row))).or_default()[side] =
                        Some((c, row.clone()));
                }
            }
        }
        let mut out = BTreeSet::new();
        for ((rel, _), [old, new]) in by_key {
            match (old, new) {
                (None, Some((c, _))) => {
                    out.insert(Requirement::Insert(self.target(rel, c)));
                }
                (Some((c, _)), None) => {
                    out.insert(Requirement::Delete(self.target(rel, c)));
                }
                (Some((from, _)), Some((to, _))) if from != to => {
                    out.insert(Requirement::Retag {
                        relation: self.target(rel, from).relation,
                        from: self.target(rel, from).constructor,
                        to: self.target(rel, to).constructor,
                    });
                }
                (Some((c, Value::Ctor(_, old))), Some((_, new))) => {
                    for (name, v) in old {
                        if new.field(&name) != Some(&v) {
                            let (t, field) = self.field(rel, c, &name);
                            out.insert(Requirement::UpdateField(t, field));
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }
}

/// Returns `db` with the rows `read` names changed: a constructor's rows removed, or a field's
/// values rotated among the constructor's rows or each changed in place, by `how`.
fn perturbed(db: &Database, read: &Read, how: usize) -> Option<Database> {
    let mut db = db.clone();
    match read {
        Read::Ctor(rel, c) => (how == 0)
            .then(|| db.remove(&(*rel, *c)).map(|_| db))
            .flatten(),
        Read::Field(rel, c, name) => {
            let rows: Vec<Value> = db.get(&(*rel, *c))?.iter().cloned().collect();
            let values: Vec<Value> = rows
                .iter()
                .map(|r| r.field(name).expect("a row holds its fields").clone())
                .collect();
            let next: Vec<Option<Value>> = match how {
                0 => (0..values.len())
                    .map(|i| Some(values[(i + 1) % values.len()].clone()))
                    .collect(),
                _ => values.iter().map(tweaked).collect(),
            };
            if next.iter().zip(&values).all(|(n, v)| n.as_ref() == Some(v)) {
                return None;
            }
            let rows = rows
                .into_iter()
                .zip(next)
                .map(|(row, v)| match (row, v) {
                    (Value::Ctor(t, fs), Some(v)) => Value::Ctor(
                        t,
                        fs.into_iter()
                            .map(|(n, old)| if n == *name { (n, v.clone()) } else { (n, old) })
                            .collect(),
                    ),
                    (row, _) => row,
                })
                .collect();
            db.insert((*rel, *c), rows);
            Some(db)
        }
    }
}

/// Returns `v` changed in place, keeping its type, or `None` for a value with no other of its
/// shape at hand.
fn tweaked(v: &Value) -> Option<Value> {
    match v {
        Value::Int(i) => Some(Value::Int(i + 1000)),
        Value::Text(t) => Some(Value::Text(format!("{t}~"))),
        Value::Ctor(c, fs) => {
            let (at, nested) = fs
                .iter()
                .enumerate()
                .find_map(|(at, (_, v))| Some((at, tweaked(v)?)))?;
            let mut fs = fs.clone();
            fs[at].1 = nested;
            Some(Value::Ctor(*c, fs))
        }
        _ => None,
    }
}

/// Returns what the role observes of `out` with `read` hidden: the result or abort, and the state
/// after, without the rows of a hidden constructor and with a hidden field blanked in every row
/// of its relation.
fn observed(out: &Result<Outcome, Abort>, read: &Read) -> Result<(Value, Database), Abort> {
    let out = out.clone()?;
    let mut db = out.db;
    match read {
        Read::Ctor(rel, c) => {
            db.remove(&(*rel, *c));
        }
        Read::Field(rel, _, name) => {
            for ((r, _), rows) in &mut db {
                if r != rel {
                    continue;
                }
                *rows = std::mem::take(rows)
                    .into_iter()
                    .map(|row| match row {
                        Value::Ctor(t, fs) => Value::Ctor(
                            t,
                            fs.into_iter()
                                .map(|(n, v)| {
                                    if n == *name {
                                        (n, Value::unit())
                                    } else {
                                        (n, v)
                                    }
                                })
                                .collect(),
                        ),
                        row => row,
                    })
                    .collect();
            }
        }
    }
    Ok((out.result, db))
}

/// Returns whether `out` is a refusal by a constraint, which the schema owner's rights judge.
fn violated(out: &Result<Outcome, Abort>) -> bool {
    matches!(out, Err(Abort::Reject(Rejection::Violated { .. })))
}

fn analysed(w: &World, text: &str) -> Option<(Adhoc, Requirements)> {
    let items = parse_adhoc(text, Options::default()).ok().ok()?;
    let adhoc = check_adhoc(w.f.compiled(), &items, &BTreeMap::new()).ok()?;
    if adhoc.items.len() != 1 || !adhoc.commands.is_empty() {
        return None;
    }
    let mut reqs = adtb_lang::permission::adhoc(w.f.compiled(), &adhoc).ok()?;
    Some((adhoc, reqs.remove(0)))
}

/// The runs of statements with stored reads hidden, each compared with the unperturbed run; a
/// pair a constraint refused either of is not counted.
#[derive(Default)]
struct Runs {
    compared: usize,
}

/// Returns the reads of `reads` whose perturbation changes what the role observes of `adhoc`
/// over the world's rows, each once.
fn witnessed<'r>(
    w: &World,
    adhoc: &Adhoc,
    reads: impl Iterator<Item = &'r Read>,
    runs: &mut Runs,
) -> Vec<Read> {
    let base = w.run(adhoc, &w.f.db);
    let mut out = Vec::new();
    for read in reads {
        for how in 0..2 {
            let Some(db) = perturbed(&w.f.db, read, how) else {
                continue;
            };
            let got = w.run(adhoc, &db);
            if violated(&base) || violated(&got) {
                continue;
            }
            runs.compared += 1;
            if observed(&base, read) != observed(&got, read) {
                out.push(read.clone());
                break;
            }
        }
    }
    out
}

/// Whatever an ad hoc statement drawn by the front fuzzer reads that the analysis leaves
/// uncharged changes neither its result, its abort nor the state it leaves, and every insert,
/// delete, field update and retag its change makes is charged.
#[test]
fn an_evaluation_depends_only_on_what_the_analysis_charges() {
    let w = World::new(SCHEMA.trim_start_matches("module app\n"), ROWS);
    let reads = w.reads();
    let seeds = profile::sweep(0..400);
    let (mut analysed_texts, mut writing, mut runs) = (0, 0, Runs::default());
    let mut under = Vec::new();
    for seed in seeds.clone() {
        let text = adhoc_text(seed);
        let Some((adhoc, reqs)) = analysed(&w, &text) else {
            continue;
        };
        analysed_texts += 1;
        let charged: BTreeSet<Requirement> = reqs.iter().cloned().collect();
        if let Ok(out) = &w.run(&adhoc, &w.f.db) {
            let written = w.written(out);
            writing += usize::from(!written.is_empty());
            for r in written.difference(&charged) {
                under.push(format!("{text}\nwrites uncharged {r:?}"));
            }
        }
        let uncharged = reads
            .iter()
            .filter(|r| !charged.contains(&w.requirement(r)));
        for read in witnessed(&w, &adhoc, uncharged, &mut runs) {
            under.push(format!("{text}\ndepends on uncharged {read:?}"));
        }
    }
    assert_eq!(Vec::<String>::new(), under);
    let n = (seeds.end - seeds.start) as usize;
    profile::assert_reached("texts analysed", analysed_texts, n * 3 / 4);
    profile::assert_reached("writing texts", writing, n / 8);
    profile::assert_reached("perturbed runs compared", runs.compared, n * 8);
}

const SUM: &str = r"
type T { id: Key ts } = A { x: Text, y: Text } | B { x: Text, y: Text } | C { x: Text, y: Text }
relation ts : T key id generate sequence
type P = { id: Key ps, name: Text }
relation ps : P key id generate sequence
type K = { id: Key ks, p: Key ps on delete cascade, note: Text }
relation ks : K key id generate sequence
";

const SUM_ROWS: &str = r#"transact
  insert ts.A { x: "a1", y: "b1" }
  insert ts.A { x: "a2", y: "b2" }
  insert ts.B { x: "a3", y: "b3" }
  insert ts.B { x: "a4", y: "b4" }
  insert ts.C { x: "a5", y: "b5" }
  insert ts.C { x: "a6", y: "b6" }
  let p = insert ps { name: "p1" }
  let q = insert ps { name: "p2" }
  insert ks { p: key p, note: "n1" }
  insert ks { p: key q, note: "n2" }
  return 0"#;

/// Statements whose charges are exact, each with the relations only its delete actions change.
const PRECISE: [(&str, &[&str]); 9] = [
    (
        "from t in ts select match t | A a -> a.x | B b -> b.x | C c -> c.y",
        &[],
    ),
    ("from t in ts select key t", &[]),
    ("count(t for t in ts.B)", &[]),
    ("exists t in ts.C", &[]),
    ("from t in ts.(A | B) select t.x", &[]),
    ("update t in ts.A set A { t with x: \"z\" }", &[]),
    ("update t in ts.A set B { t with x: \"r\", y: \"s\" }", &[]),
    ("insert ts.C { x: \"c\", y: \"d\" }", &[]),
    ("delete p in ps where p.name == \"p1\"", &["ks"]),
];

/// The analysis charges exactly the stored reads a statement's outcome depends on and the writes
/// its change makes, for a matched row's used fields, an unchanged carry, a range, count and
/// exists over keys, a range over part of a sum and a cascade's implicit deletes; a delete
/// action's changes and reads are charged to no one.
#[test]
fn precise_forms_are_charged_and_admitted_by_exactly_what_they_read_and_write() {
    let w = World::new(SUM, SUM_ROWS);
    let mut wrong = Vec::new();
    let mut runs = Runs::default();
    let mut grant_sets = 0;
    for (text, implicit) in PRECISE {
        let (adhoc, reqs) = analysed(&w, text).unwrap_or_else(|| panic!("`{text}` checks"));
        let implicit: BTreeSet<RelId> = implicit.iter().map(|r| w.f.rel(r)).collect();
        let reads: Vec<Read> = w
            .reads()
            .into_iter()
            .filter(|r| !implicit.contains(&r.rel()))
            .collect();
        let read: BTreeSet<Requirement> = witnessed(&w, &adhoc, reads.iter(), &mut runs)
            .iter()
            .map(|r| w.requirement(r))
            .collect();
        let out = w
            .run(&adhoc, &w.f.db)
            .unwrap_or_else(|a| panic!("`{text}` runs: {a:?}"));
        let written: Vec<Requirement> = w
            .written(&out)
            .into_iter()
            .filter(|r| {
                !implicit
                    .iter()
                    .any(|&rel| Some(w.target(rel, 0).relation) == r.relation())
            })
            .collect();
        let ranged = written.iter().filter_map(|r| match *r {
            Requirement::UpdateField(t, _) | Requirement::Delete(t) => Some(t),
            Requirement::Retag { relation, from, .. } => Some(Target {
                relation,
                constructor: from,
            }),
            _ => None,
        });
        let ranged: Vec<Requirement> = ranged.map(Requirement::ReadConstructor).collect();
        let expected: BTreeSet<Requirement> =
            read.into_iter().chain(ranged).chain(written).collect();
        let charged: BTreeSet<Requirement> =
            reqs.iter().filter(|r| !w.is_key_read(r)).cloned().collect();
        for grants in 0..125 {
            let (g, covered) = w.grant_set(grants);
            let missing: BTreeSet<Requirement> = reqs
                .missing(&g, Mode::Ordinary)
                .into_iter()
                .filter(|r| !w.is_key_read(r))
                .collect();
            let uncovered: BTreeSet<Requirement> = expected.difference(&covered).cloned().collect();
            if missing != uncovered {
                wrong.push(format!(
                    "{text} under read grants {grants}\n  missing {missing:?}\n  uncovered {uncovered:?}"
                ));
            }
            grant_sets += 1;
        }
        if charged != expected {
            wrong.push(format!(
                "{text}\n  charged, not observed: {:?}\n  observed, not charged: {:?}",
                charged.difference(&expected).collect::<Vec<_>>(),
                expected.difference(&charged).collect::<Vec<_>>()
            ));
        }
    }
    assert_eq!(Vec::<String>::new(), wrong);
    profile::assert_reached("perturbed runs compared", runs.compared, PRECISE.len() * 10);
    assert_eq!(PRECISE.len() * 125, grant_sets);
}
