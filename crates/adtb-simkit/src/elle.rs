//! Run the isolation checker's list-append workload on the engine, through its public calls.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use adtb::runtime::{ThreadId, Threads};
use adtb::{
    AdHocTransaction, Database, Error, InTxn, OpenError, OpenOptions, Registered, Registration,
    RejectReason, RetryReason, TxnCallOptions, TxnId, TxnOptions, Value,
};
use adtb_harness::process;
use adtb_isolation::Aborted;
use adtb_lang::textual::Form;
use adtb_sim::{Config, Sim, SimRuntime};

use crate::api;
use crate::open::{self, FILE};

/// The workload's schema: the elements of each key's list, one row per element at its position,
/// and counters keyed by id.
pub const SCHEMA: &str = "module elle
add module elle
add type Elem = { id: Key elems, k: I64, pos: I64, e: I64 }
add relation elems : Elem key id generate sequence unique (k, e) unique (k, pos)
add type Counter = { id: I64, n: I64 }
add relation counters : Counter key id
";

/// The workload's calls: `append` adds an element at the end of a key's list, `read_elems`
/// returns a key's elements with their positions.
pub const CALLS: &str = "import elle
txn append (k: I64, e: I64) -> Key elems =
  transact
    let n = count(x for x in elems where x.k == k)
    let x = insert elems { k: k, pos: n, e: e }
    return key x
query read_elems (k: I64) -> Set { pos: I64, e: I64 } =
  from x in elems where x.k == k select { pos: x.pos, e: x.e }
";

const BUNDLE: &str = "elle";

/// The constraint two appends at one position of a list violate.
const POSITIONS: &str = "elle::elems.unique(k, pos)";

/// How a transaction of the workload begins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    ReadWrite,
    ReadOnly,
    /// Read-only, waiting for a safe snapshot before its first read.
    Deferrable,
}

impl Kind {
    fn options(self) -> TxnOptions {
        TxnOptions {
            read_only: self != Kind::ReadWrite,
            deferrable: self == Kind::Deferrable,
            ..TxnOptions::default()
        }
    }
}

pub type Txn<'a> = AdHocTransaction<'a, SimRuntime>;

/// The workload's database on the simulator.
#[derive(Debug)]
pub struct Elems {
    db: Database<SimRuntime>,
    calls: Registered<SimRuntime>,
    append: u32,
    read: u32,
}

/// Returns the options that open the workload's database, creating it if it does not exist.
pub fn options() -> OpenOptions {
    OpenOptions {
        registrations: vec![Registration::new(BUNDLE, CALLS)],
        ..open::options(1)
    }
}

impl Elems {
    /// Open the workload's database in [`FILE`] with `options`.
    ///
    /// Preconditions: `options` register [`CALLS`] as [`options`] does.
    pub fn open(rt: &SimRuntime, options: &OpenOptions) -> Result<Elems, OpenError> {
        let db = open::open(rt, FILE, &adtb::Log::new([SCHEMA]), options)?;
        let calls = db
            .registered(BUNDLE)
            .expect("the workload's calls are registered");
        let ordinal = |name| calls.ordinal(name).expect("a call of the workload");
        Ok(Elems {
            append: ordinal("append"),
            read: ordinal("read_elems"),
            calls,
            db,
        })
    }

    pub fn client(&self) -> Client<'_> {
        Client(self)
    }

    pub fn database(&self) -> &Database<SimRuntime> {
        &self.db
    }

    pub fn begin(&self, kind: Kind) -> Result<Txn<'_>, Error> {
        self.db.begin(&kind.options())
    }

    /// Append `e` to the list of `k` in `tx` with the registered `append`.
    pub fn append(&self, tx: &mut Txn<'_>, k: i64, e: i64) -> Result<(), Error> {
        let args = [Value::Int(k), Value::Int(e)];
        match tx.call(&self.calls, self.append, &args, &TxnCallOptions::default())? {
            InTxn::Pending(_) => Ok(()),
            InTxn::Rows(_) => panic!("`append` is a transaction"),
        }
    }

    /// Returns the list of `k` as `tx` sees it, oldest element first, with the registered
    /// `read_elems`.
    pub fn read(&self, tx: &mut Txn<'_>, k: i64) -> Result<Vec<i64>, Error> {
        let args = [Value::Int(k)];
        let InTxn::Rows(rows) =
            tx.call(&self.calls, self.read, &args, &TxnCallOptions::default())?
        else {
            panic!("`read_elems` is a query");
        };
        let Form::Record(fields) = rows.form().clone() else {
            panic!("`read_elems` returns records");
        };
        let field = |name| {
            fields
                .iter()
                .position(|(f, _)| f == name)
                .expect("a field of `read_elems`")
        };
        let (pos, e) = (field("pos"), field("e"));
        let mut list: Vec<(i64, i64)> = rows
            .map(|row| match row.expect("a row") {
                Value::Record(r) => (int(&r[pos]), int(&r[e])),
                v => panic!("{v:?} is not a record"),
            })
            .collect();
        list.sort_unstable();
        Ok(list.into_iter().map(|(_, e)| e).collect())
    }

    /// Returns the list of every key of `keys` as one read-only transaction sees them.
    pub fn lists(&self, keys: std::ops::Range<i64>) -> Vec<Vec<i64>> {
        let mut tx = self.begin(Kind::ReadOnly).expect("a reader begins");
        let lists = keys
            .map(|k| self.read(&mut tx, k).expect("a read-only read"))
            .collect();
        tx.commit(None).expect("a reader commits");
        lists
    }

    /// Returns the rows `verify` reports over the whole database, printed.
    pub fn verify(&self) -> Vec<String> {
        api::printed(self.db.verify(None).expect("verify runs"))
    }
}

fn int(v: &Value) -> i64 {
    match v {
        Value::Int(i) => *i,
        v => panic!("{v:?} is not an I64"),
    }
}

/// Returns `Aborted` for an error that aborts a workload transaction: a serialization failure,
/// or a violation of `unique (k, pos)` by two appends at one position.
///
/// Panic at any other error.
pub fn aborted(e: Error) -> Aborted {
    match &e {
        Error::Retry {
            reason: RetryReason::Serialization { .. },
        } => Aborted,
        Error::Rejected {
            reason: RejectReason::Violated { constraint, .. },
        } if constraint == POSITIONS => Aborted,
        _ => panic!("a workload transaction fails: {e:?}"),
    }
}

/// The isolation checker's database over [`Elems`], each transaction read-write.
#[derive(Clone, Copy, Debug)]
pub struct Client<'d>(&'d Elems);

impl<'d> adtb_isolation::Database for Client<'d> {
    type Txn = Txn<'d>;

    fn begin(&mut self) -> Txn<'d> {
        self.0.begin(Kind::ReadWrite).expect("a transaction begins")
    }

    fn append(&mut self, txn: &mut Txn<'d>, key: i64, elem: i64) -> Result<(), Aborted> {
        self.0.append(txn, key, elem).map_err(aborted)
    }

    fn read(&mut self, txn: &mut Txn<'d>, key: i64) -> Result<Vec<i64>, Aborted> {
        self.0.read(txn, key).map_err(aborted)
    }

    fn commit(&mut self, txn: Txn<'d>) -> Result<(), Aborted> {
        txn.commit(None).map(drop).map_err(aborted)
    }

    fn abort(&mut self, txn: Txn<'d>) {
        txn.rollback().expect("a rollback");
    }
}

/// A request to a [`Worker`]'s transaction.
#[derive(Debug)]
pub enum Req {
    Append(i64, i64),
    Read(i64),
    /// Run ad hoc text; the reply holds the integers of its last result.
    Text(String),
    Commit,
    Abort,
}

pub type Reply = Result<Vec<i64>, Error>;

#[derive(Debug)]
struct Mailbox {
    req: Mutex<Option<Req>>,
    reply: Mutex<Option<Reply>>,
    driver: ThreadId,
}

/// A transaction running on its own simulated thread, driven one request at a time. A failed
/// request ends the transaction.
#[derive(Debug)]
pub struct Worker {
    mailbox: Arc<Mailbox>,
    thread: ThreadId,
    id: TxnId,
}

fn take<T>(rt: &SimRuntime, slot: &Mutex<Option<T>>) -> T {
    loop {
        if let Some(v) = slot.lock().unwrap().take() {
            return v;
        }
        rt.park();
    }
}

impl Worker {
    /// Start a transaction of `kind` on a thread of its own. Returns once it has begun.
    pub fn start(rt: &SimRuntime, elems: &Arc<Elems>, kind: Kind) -> Worker {
        let mailbox = Arc::new(Mailbox {
            req: Mutex::new(None),
            reply: Mutex::new(None),
            driver: rt.current(),
        });
        let (m, e, rt2) = (mailbox.clone(), elems.clone(), rt.clone());
        let thread = rt.spawn("txn", move || Worker::serve(&e, &rt2, &m, kind));
        let began = take(rt, &mailbox.reply).expect("the transaction begins");
        Worker {
            mailbox,
            thread,
            id: TxnId(began[0]),
        }
    }

    fn serve(elems: &Elems, rt: &SimRuntime, m: &Mailbox, kind: Kind) {
        let reply = |r: Reply| {
            *m.reply.lock().unwrap() = Some(r);
            rt.unpark(m.driver);
        };
        let mut tx = match elems.begin(kind) {
            Ok(tx) => tx,
            Err(e) => return reply(Err(e)),
        };
        reply(Ok(vec![tx.id().0]));
        loop {
            let r = match take(rt, &m.req) {
                Req::Append(k, e) => elems.append(&mut tx, k, e).map(|()| vec![]),
                Req::Read(k) => elems.read(&mut tx, k),
                Req::Text(text) => {
                    tx.query_text(&text, &[], &TxnCallOptions::default())
                        .map(|results| {
                            results
                                .into_iter()
                                .last()
                                .map(|rows| rows.map(|v| int(&v.expect("a row"))).collect())
                                .unwrap_or_default()
                        })
                }
                Req::Commit => return reply(tx.commit(None).map(|_| vec![])),
                Req::Abort => return reply(tx.rollback().map(|()| vec![])),
            };
            let failed = r.is_err();
            reply(r);
            if failed {
                return;
            }
        }
    }

    pub fn thread(&self) -> ThreadId {
        self.thread
    }

    /// Returns the id that names the transaction in its errors.
    pub fn id(&self) -> TxnId {
        self.id
    }

    /// Send `req` and return without waiting for the reply.
    pub fn send(&self, rt: &SimRuntime, req: Req) {
        *self.mailbox.req.lock().unwrap() = Some(req);
        rt.unpark(self.thread);
    }

    pub fn reply(&self, rt: &SimRuntime) -> Reply {
        take(rt, &self.mailbox.reply)
    }

    /// Returns the reply if the worker has answered.
    pub fn try_reply(&self) -> Option<Reply> {
        self.mailbox.reply.lock().unwrap().take()
    }

    pub fn call(&self, rt: &SimRuntime, req: Req) -> Reply {
        self.send(rt, req);
        self.reply(rt)
    }
}

/// Returns the files of a new workload database as [`options`] creates it, built once per
/// process.
pub fn created() -> BTreeMap<String, Vec<u8>> {
    static FILES: OnceLock<BTreeMap<String, Vec<u8>>> = OnceLock::new();
    FILES
        .get_or_init(|| {
            let mut sim = Sim::new(0, Config::default());
            process::run(&mut sim, |rt| {
                drop(Elems::open(&rt, &options()).expect("the workload's database opens"));
            });
            sim.files()
        })
        .clone()
}

/// Run `body` in a simulated process seeded `seed` over a new workload database opened with
/// `options`, then check that `verify` finds nothing. Returns what `body` returns.
///
/// Preconditions: `options` create the file [`options`] creates.
pub fn simulate_with<T>(
    seed: u64,
    options: &OpenOptions,
    body: impl FnOnce(&SimRuntime, &Arc<Elems>) -> T,
) -> T {
    let mut sim = Sim::with_files(seed, Config::default(), created());
    process::run(&mut sim, |rt| {
        let elems = Arc::new(Elems::open(&rt, options).expect("the workload's database opens"));
        let out = body(&rt, &elems);
        assert_eq!(Vec::<String>::new(), elems.verify(), "seed {seed}");
        out
    })
}

/// Run `body` as [`simulate_with`] does, with [`options`].
pub fn simulate<T>(seed: u64, body: impl FnOnce(&SimRuntime, &Arc<Elems>) -> T) -> T {
    simulate_with(seed, &options(), body)
}

#[cfg(test)]
mod tests {
    use adtb_isolation::{Aborted, Database as _};

    use super::*;

    #[test]
    fn appends_read_back_in_order_inside_their_transaction_and_after_it_commits() {
        simulate(1, |_, elems| {
            let mut tx = elems.begin(Kind::ReadWrite).unwrap();
            elems.append(&mut tx, 1, 30).unwrap();
            elems.append(&mut tx, 1, 10).unwrap();
            elems.append(&mut tx, 2, 20).unwrap();
            assert_eq!(Ok(vec![30, 10]), elems.read(&mut tx, 1));
            tx.commit(None).unwrap();
            assert_eq!(vec![vec![], vec![30, 10], vec![20]], elems.lists(0..3));
        });
    }

    #[test]
    fn a_read_only_or_deferrable_transaction_reads_and_refuses_an_append() {
        simulate(2, |_, elems| {
            for kind in [Kind::ReadOnly, Kind::Deferrable] {
                let mut tx = elems.begin(kind).unwrap();
                assert_eq!(Ok(vec![]), elems.read(&mut tx, 1), "{kind:?}");
                assert_eq!(
                    Err(Error::Rejected {
                        reason: RejectReason::Invalid {
                            detail: "a read-only transaction does not write".to_owned()
                        }
                    }),
                    elems.append(&mut tx, 1, 1),
                    "{kind:?}"
                );
                tx.commit(None).unwrap();
            }
        });
    }

    #[test]
    fn of_two_appends_at_one_position_only_the_first_to_commit_commits() {
        simulate(3, |_, elems| {
            let mut db = elems.client();
            let mut late = db.begin();
            let mut first = db.begin();
            db.append(&mut first, 1, 1).unwrap();
            assert_eq!(Ok(()), db.commit(first));
            assert_eq!(Err(Aborted), db.append(&mut late, 1, 2));
            drop(late);
            assert_eq!(vec![vec![1]], elems.lists(1..2));
        });
    }

    #[test]
    fn an_append_at_a_taken_position_violates_unique_positions_and_is_aborted() {
        simulate(5, |_, elems| {
            let db = elems.database();
            db.query_text(
                "insert elems { k: 1, pos: 1, e: 9 }",
                &[],
                &adtb::CallOptions::default(),
            )
            .unwrap();
            let elem = |id, e| Value::Ctor(0, [id, 1, 1, e].map(Value::Int).into());
            let pair = |a, b| Value::Record([a, b].into());
            let mut tx = elems.begin(Kind::ReadWrite).unwrap();
            assert_eq!(
                Err(Error::Rejected {
                    reason: RejectReason::Violated {
                        constraint: POSITIONS.to_owned(),
                        report: vec![pair(elem(1, 9), elem(2, 2)), pair(elem(2, 2), elem(1, 9)),],
                        redacted: 0,
                    }
                }),
                elems.append(&mut tx, 1, 2)
            );
            drop(tx);
            let mut client = elems.client();
            let mut tx = client.begin();
            assert_eq!(Err(Aborted), client.append(&mut tx, 1, 3));
        });
    }

    #[test]
    fn a_commit_the_serializer_refuses_is_aborted() {
        simulate(4, |_, elems| {
            let mut db = elems.client();
            let mut a = db.begin();
            let mut b = db.begin();
            for t in [&mut a, &mut b] {
                assert_eq!(Ok(vec![]), db.read(t, 1));
                assert_eq!(Ok(vec![]), db.read(t, 2));
            }
            db.append(&mut a, 1, 1).unwrap();
            db.append(&mut b, 2, 1).unwrap();
            assert_eq!(Ok(()), db.commit(a));
            assert_eq!(Err(Aborted), db.commit(b));
            assert_eq!(vec![vec![1], vec![]], elems.lists(1..3));
        });
    }
}
