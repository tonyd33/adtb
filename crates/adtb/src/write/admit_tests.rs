use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use adtb_lang::schema::ValueDef;
use adtb_lang::types::RelId;
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_testkit::Rng;
use adtb_testkit::fixture::Fixture;
use adtb_testkit::seeds::{assert_reached, sweep};

use crate::steps::Step;
use crate::test_world::write_path::WritePath;
use crate::write::{Error, Rejection};

/// The value limit the search's database lowers to, in place of 2^31 - 1.
const VALUE_LIMIT: usize = 6000;

/// The length of the key that finds the key limit, under [`VALUE_LIMIT`].
const KEY_PROBE: usize = 4000;

/// The length of each text of the record field that finds its encoding's overhead: under
/// [`VALUE_LIMIT`] each, over it together.
const PART_PROBE: usize = 4000;

/// The shortest text of the record field at the value limit, its length prefix as long as
/// [`PART_PROBE`]'s.
const PART_LEAST: usize = 1000;

/// The longest text of a short row.
const SHORT: u64 = 4;

/// The longest text that stays inline in a row's smallest form.
const INLINE: usize = 16;

/// The widest fixed field a drawn type holds: `P8`, of 2^8 `I64` leaves.
const WIDEST: u32 = 8;

/// A drawn record type: `id: I64`, then its fields in declaration order.
struct Drawn {
    fields: Vec<Field>,
    texts: usize,
}

#[derive(Clone, Copy)]
enum Field {
    K,
    V,
    /// `q: Q`, a record of two texts.
    Q,
    T(usize),
    /// The `j`th fixed field, of type `P{b}`, 2^b `I64` leaves.
    W {
        j: usize,
        b: u32,
    },
}

/// Returns the declarations of `P1` to `P{WIDEST}`, `P{b}` two `P{b - 1}`, `P0` being `I64`.
fn wide_types() -> String {
    let p = |b: u32| match b {
        0 => "I64".to_owned(),
        b => format!("P{b}"),
    };
    (1..=WIDEST)
        .map(|b| format!("type P{b} = {{ a: {}, b: {} }}\n", p(b - 1), p(b - 1)))
        .collect()
}

/// Returns the literal of a `P{b}` whose every leaf is 0.
fn wide(b: u32) -> String {
    match b {
        0 => "0".to_owned(),
        b => format!("{{ a: {}, b: {} }}", wide(b - 1), wide(b - 1)),
    }
}

impl Drawn {
    /// Draw a type whose fixed part is close below the row limit of the test world's 4 KiB page,
    /// so that its texts at [`SHORT`] fit and at [`INLINE`] do not.
    fn draw(rng: &mut Rng) -> Drawn {
        let texts = 30 + rng.below(11) as usize;
        let mut leaves = 440 + rng.below(21);
        let mut widths = Vec::new();
        for b in (0..=WIDEST).rev() {
            while leaves >= 1 << b && (b == 0 || rng.below(4) != 0) {
                widths.push(b);
                leaves -= 1 << b;
            }
        }
        let mut fields: Vec<Field> = [Field::K, Field::V, Field::Q]
            .into_iter()
            .chain((0..texts).map(Field::T))
            .chain(
                widths
                    .into_iter()
                    .enumerate()
                    .map(|(j, b)| Field::W { j, b }),
            )
            .collect();
        for i in (1..fields.len()).rev() {
            fields.swap(i, rng.below(i as u64 + 1) as usize);
        }
        Drawn { fields, texts }
    }

    fn decls(&self) -> String {
        let fields: Vec<String> = self
            .fields
            .iter()
            .map(|f| match f {
                Field::K => "k: Text".to_owned(),
                Field::V => "v: Text".to_owned(),
                Field::Q => "q: Q".to_owned(),
                Field::T(i) => format!("t{i}: Text"),
                Field::W { j, b: 0 } => format!("w{j}: I64"),
                Field::W { j, b } => format!("w{j}: P{b}"),
            })
            .collect();
        format!(
            "{}type Q = {{ a: Text, b: Text }}\ntype R = {{ id: I64, {} }}\nrelation rs : R key id index (k)",
            wide_types(),
            fields.join(", ")
        )
    }

    /// Returns a row of id `id` whose every text is at most [`SHORT`] long.
    fn short(&self, rng: &mut Rng, id: i64) -> Row {
        let mut len = || rng.below(SHORT + 1) as usize;
        Row {
            id,
            k: len(),
            v: len(),
            q: (len(), len()),
            t: (0..self.texts).map(|_| len()).collect(),
        }
    }

    /// Returns the fields of `row`, its fixed fields too when `whole`.
    fn fields(&self, row: &Row, whole: bool) -> String {
        let text = |n: usize| format!("\"{}\"", "y".repeat(n));
        let fields: Vec<String> = self
            .fields
            .iter()
            .filter_map(|f| match f {
                Field::K => Some(format!("k: {}", text(row.k))),
                Field::V => Some(format!("v: {}", text(row.v))),
                Field::Q => Some(format!(
                    "q: {{ a: {}, b: {} }}",
                    text(row.q.0),
                    text(row.q.1)
                )),
                Field::T(i) => Some(format!("t{i}: {}", text(row.t[*i]))),
                Field::W { j, b } => whole.then(|| format!("w{j}: {}", wide(*b))),
            })
            .collect();
        fields.join(", ")
    }

    fn insert(&self, row: &Row) -> String {
        format!("insert rs {{ id: {}, {} }}", row.id, self.fields(row, true))
    }

    fn update(&self, row: &Row) -> String {
        format!(
            "update x in rs where x.id == {} set R {{ x with {} }}",
            row.id,
            self.fields(row, false)
        )
    }
}

/// A row by the lengths of its texts, each all `y`.
#[derive(Clone)]
struct Row {
    id: i64,
    k: usize,
    v: usize,
    q: (usize, usize),
    t: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Limit {
    Row,
    Value,
    /// The value limit of a record field's whole value.
    Whole,
    Key,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Side {
    At,
    Past,
}

/// One case's database and what it has drawn.
struct Case<'w> {
    w: &'w WritePath<SimRuntime>,
    rng: Rng,
    drawn: Drawn,
    rs: RelId,
    next: i64,
    /// The heap writes and take-backs the hook saw since the last statement began.
    writes: Arc<AtomicUsize>,
}

impl Case<'_> {
    /// Run `subject` in one statement beside up to two short inserts, in a drawn order, then
    /// commit it if it succeeds. Returns its outcome and the heap writes it made.
    fn run(&mut self, subject: String) -> (Result<(), Error>, usize) {
        let mut items = vec![subject];
        for _ in 0..self.rng.below(3) {
            let row = self.drawn.short(&mut self.rng, self.next);
            self.next += 1;
            let at = self.rng.below(items.len() as u64 + 1) as usize;
            items.insert(at, self.drawn.insert(&row));
        }
        let text = match items.as_slice() {
            [one] => one.clone(),
            many => format!("({})", many.join(", ")),
        };
        let (terms, item, _) = self.w.prepare(&text, &[]);
        let db = self.w.db();
        let tx = db.begin().expect("a transaction begins");
        self.writes.store(0, Ordering::SeqCst);
        let out = db.statement(&tx, &terms, item, &[], 0).map(drop);
        let writes = self.writes.load(Ordering::SeqCst);
        match out {
            Ok(()) => drop(db.commit(tx).expect("an admitted statement commits")),
            Err(_) => db.abort(tx).expect("an abort"),
        }
        (out, writes)
    }

    /// Returns the statement writing `row` as a new row or over the row of its id.
    fn subject(&mut self, mut row: Row, over: Option<i64>) -> String {
        match over {
            Some(id) => {
                row.id = id;
                self.drawn.update(&row)
            }
            None => {
                row.id = self.next;
                self.next += 1;
                self.drawn.insert(&row)
            }
        }
    }

    fn row_too_large(&self, size: usize, limit: usize) -> Error {
        Error::Rejected(Rejection::RowTooLarge {
            relation: self.rs,
            ctor: 0,
            size,
            limit,
        })
    }

    fn key_too_large(&self, size: usize, limit: usize) -> Error {
        Error::Rejected(Rejection::KeyTooLarge {
            relation: self.rs,
            ctor: 0,
            index: "index (k)".into(),
            size,
            limit,
        })
    }

    /// Run `row` past a limit to find it. Returns the size and limit of its refusal.
    fn probe(&mut self, limit: Limit, row: Row, over: Option<i64>) -> (usize, usize) {
        let s = self.subject(row, over);
        let (out, writes) = self.run(s);
        assert_eq!(0, writes, "a probe past the {limit:?} limit writes nothing");
        match (limit, out) {
            (
                Limit::Row | Limit::Whole,
                Err(Error::Rejected(Rejection::RowTooLarge {
                    relation,
                    ctor: 0,
                    size,
                    limit,
                })),
            )
            | (
                Limit::Key,
                Err(Error::Rejected(Rejection::KeyTooLarge {
                    relation,
                    ctor: 0,
                    size,
                    limit,
                    ..
                })),
            ) if relation == self.rs => (size, limit),
            (limit, out) => panic!("a probe past the {limit:?} limit is refused by it: {out:?}"),
        }
    }

    /// Run `at`, a row at `limit`, then the row one byte past it, `grow` adding the byte, which
    /// must be refused with `refusal`.
    fn both_sides(
        &mut self,
        limit: Limit,
        at: Row,
        grow: impl Fn(&mut Row),
        over: Option<i64>,
        refusal: Error,
        reached: &mut BTreeMap<(Limit, Side), usize>,
    ) {
        let s = self.subject(at.clone(), over);
        let (out, writes) = self.run(s);
        assert_eq!(Ok(()), out, "a row at the {limit:?} limit is admitted");
        assert!(0 < writes, "a row at the {limit:?} limit is written");
        *reached.entry((limit, Side::At)).or_default() += 1;
        let mut past = at;
        grow(&mut past);
        let s = self.subject(past, over);
        let (out, writes) = self.run(s);
        assert_eq!(Err(refusal), out, "a row one byte past the {limit:?} limit");
        assert_eq!(
            0, writes,
            "a row past the {limit:?} limit is refused before any heap write"
        );
        *reached.entry((limit, Side::Past)).or_default() += 1;
    }
}

/// A statement whose change set holds a new row at a limit (the row limit of its stored form,
/// the value limit of a text field or of a record field's whole value, the key limit of its
/// index key) is admitted and
/// written; one byte past it the statement is refused with the limit's exact refusal before any
/// heap write, whatever rows the statement writes beside it and in whatever order, by insert or
/// update, over drawn record types.
#[test]
fn a_row_past_each_limit_is_refused_before_any_heap_write_and_one_at_it_is_written() {
    const CASES: std::ops::Range<u64> = 0..4;
    let reached: Arc<Mutex<BTreeMap<(Limit, Side), usize>>> = Arc::default();
    let row_limits: Arc<Mutex<Vec<usize>>> = Arc::default();
    for seed in sweep(CASES) {
        let mut rng = Rng::new(seed);
        let drawn = Drawn::draw(&mut rng);
        let f = Fixture::new(&drawn.decls());
        let (reached, row_limits) = (reached.clone(), row_limits.clone());
        Sim::new(seed, Config::default()).run(move |rt| {
            let w = WritePath::of_value_limit(&rt, &f, VALUE_LIMIT as u64);
            let m = w.schema().module_named("app").expect("module app");
            let Some(&ValueDef::Relation(rs)) = w.schema().module(m).values.get("rs") else {
                panic!("app holds relation rs")
            };
            let writes = Arc::new(AtomicUsize::new(0));
            let counted = writes.clone();
            w.db().set_step_hook(Arc::new(move |s| {
                if matches!(
                    s,
                    Step::WriteInserted | Step::WriteChanged(_) | Step::TakeBack
                ) {
                    counted.fetch_add(1, Ordering::SeqCst);
                }
            }));
            let mut case = Case {
                w: &w,
                rng,
                drawn,
                rs,
                next: 1,
                writes,
            };
            let base = case.drawn.short(&mut case.rng, 0);
            let (out, _) = case.run(case.drawn.insert(&base));
            assert_eq!(Ok(()), out, "the base row is admitted");
            let mut reached = reached.lock().unwrap();
            let mut limits = [Limit::Row, Limit::Value, Limit::Whole, Limit::Key];
            for i in (1..limits.len()).rev() {
                limits.swap(i, case.rng.below(i as u64 + 1) as usize);
            }
            for limit in limits {
                let over = (case.rng.below(2) == 0).then_some(0);
                let short = case.drawn.short(&mut case.rng, 0);
                match limit {
                    Limit::Row => {
                        let mut wide = short;
                        wide.k = INLINE;
                        wide.v = INLINE;
                        wide.t.iter_mut().for_each(|t| *t = INLINE);
                        let (size, limit) = case.probe(Limit::Row, wide.clone(), over);
                        row_limits.lock().unwrap().push(limit);
                        let mut excess = size - limit;
                        for t in &mut wide.t {
                            let cut = excess.min(*t);
                            *t -= cut;
                            excess -= cut;
                        }
                        assert_eq!(0, excess, "the drawn texts cover the row's excess");
                        let refusal = case.row_too_large(limit + 1, limit);
                        let grow = |r: &mut Row| {
                            let t = r.t.iter_mut().find(|t| **t < INLINE);
                            *t.expect("a text was cut") += 1;
                        };
                        case.both_sides(Limit::Row, wide, grow, over, refusal, &mut reached);
                    }
                    Limit::Value => {
                        let mut long = short;
                        long.v = VALUE_LIMIT;
                        let refusal = case.row_too_large(VALUE_LIMIT + 1, VALUE_LIMIT);
                        let grow = |r: &mut Row| r.v += 1;
                        case.both_sides(Limit::Value, long, grow, over, refusal, &mut reached);
                    }
                    Limit::Whole => {
                        let mut long = short;
                        long.q = (PART_PROBE, PART_PROBE);
                        let (size, limit) = case.probe(Limit::Whole, long.clone(), over);
                        assert_eq!(VALUE_LIMIT, limit, "a record field has the value limit");
                        let parts = limit - (size - 2 * PART_PROBE);
                        let a =
                            PART_LEAST + case.rng.below((parts - 2 * PART_LEAST) as u64) as usize;
                        long.q = (a, parts - a);
                        let refusal = case.row_too_large(VALUE_LIMIT + 1, VALUE_LIMIT);
                        let grow = |r: &mut Row| r.q.0 += 1;
                        case.both_sides(Limit::Whole, long, grow, over, refusal, &mut reached);
                    }
                    Limit::Key => {
                        let mut long = short;
                        long.k = KEY_PROBE;
                        let (size, limit) = case.probe(Limit::Key, long.clone(), over);
                        long.k = limit - (size - KEY_PROBE);
                        let refusal = case.key_too_large(limit + 1, limit);
                        let grow = |r: &mut Row| r.k += 1;
                        case.both_sides(Limit::Key, long, grow, over, refusal, &mut reached);
                    }
                }
            }
        });
    }
    let row_limits = row_limits.lock().unwrap();
    assert!(
        row_limits.windows(2).all(|w| w[0] == w[1]),
        "every drawn type has the database's one row limit: {row_limits:?}"
    );
    let reached = reached.lock().unwrap();
    let ran = sweep(CASES).count();
    for limit in [Limit::Row, Limit::Value, Limit::Whole, Limit::Key] {
        for side in [Side::At, Side::Past] {
            let n = reached.get(&(limit, side)).copied().unwrap_or(0);
            assert_reached(&format!("{limit:?} {side:?}"), n, ran);
        }
    }
}
