# Test surface

`adtb::testing` is what the test harness observes of a running engine
that no public interface shows: quarantine marks, damage evidence, log
positions, maintenance counters, the background scrub's cursor and the
findings the engine holds. It exists only with the `testing` feature of
`adtb`, which only the tests of `adtb-sim` enable.

## 1. Entry points

```text
Database::testing(&self) -> Testing<'_, R>     // an open database
Testing::probe(self) -> Probe<'_, R>           // the database's pager
Testing::txn_probe(self) -> TxnProbe<'_, R>    // the database's transactions
Probe::of(&Pager<R>) -> Probe<'_, R>           // a pager a test built
TxnProbe::of(&Transactions<R>) -> TxnProbe<'_, R>
```

| Entry | Observations |
| --- | --- |
| `Testing` | `next_xid`, `commit_log_damage`, `charged` (the vacuum pacer's steps), `rebuild_waits`, `vacuum_yields`, `reindex_threads`, `scrub` (the saved cursor), `held_findings` |
| `Probe` | `marks`, `marked`, `refusals`, `records`, `damage_in_log`, `logged_commits`, `appended`, `durable`, `checkpoint`, `dirty`, `walks` |
| `TxnProbe` | `next_xid`, `commit_log_damage` |

## 2. Contract

Postconditions:

- Every observation returns a value built at the call: a set, a map, a
  count, an LSN, an id or a flag. No observation returns a reference
  into the engine, and no type of this module holds one past the
  borrow of the database or the pager it was made from.
- Every observation but `damage_in_log` and `logged_commits` reads
  memory only and changes neither the engine's state nor what a later
  call returns.
- `damage_in_log` and `logged_commits` read the retained log as
  `verify` does: they pin the log while they read, list its directory
  and open its segments through the runtime, which records which
  segments are open and, under the simulator, takes fault draws a
  later operation would otherwise take. They return nothing when the
  segments cannot be listed or opened.
- `marked(object)` holds when the object or the whole database is
  marked, as admission of a call checks it.

Not promised:

- Consistency between two observations. Each is a snapshot of one fact
  at the call, ordered against the engine's threads only by the lock
  that guards that fact.
- The representation behind an observation: `dirty` answers the LSN
  that first dirtied each page, whatever structure keeps it, and
  `records` answers what is recorded, whichever collection
  holds it.

Invariant: no build of a package without its tests has the `testing`
feature. No package of the repository has a normal or build dependency
on `adtb` that enables it, no feature of a package enables it, and
`adtb`'s default features exclude it. `check` fails otherwise.

## 3. Ownership

The module owns the mapping from engine internals to its answers. When
storage, transactions or maintenance change how they keep a fact, this
module changes and no test does. The answers carry the engine's
vocabulary types (`Pid`, `Lsn`, `Damage`, `Quarantine`, `PageError`,
`Xid`, `LogFinding`, `vacuum::Charged`) and this module's own `Records`
(one snapshot, read through `pages`, `log` and `full`), never a pager,
a heap, an index or the write path.
