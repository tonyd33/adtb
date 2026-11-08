# Test surface

`adtb::testing` is what the test harness observes of a running engine
that no public interface shows: quarantine marks, damage evidence, log
positions, maintenance counters, the background scrub's cursor and the
findings the engine holds. It also holds the controls a test drives the
engine with where no public call reaches: faults planted beneath the
checks of the write path, flushes, holds and hooks, and the settings an
open takes only in tests. It exists only with the `testing` feature of
`adtb`, which only the tests of `adtb-sim` enable.

The harness the simulator's tests share, crashes, opens, component
worlds and byte damage, is `adtb-simkit` (§5).

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
| `Testing` | `next_xid`, `commit_log_damage`, `charged` (the vacuum pacer's steps), `rebuild_waits`, `vacuum_yields`, `tracked` (the transactions the serializer tracks), `snapshot` (the snapshot a transaction reads, `None` once it has ended), `reindex_threads`, `scrub` (the saved cursor), `held_findings` |
| `Probe` | `marks`, `marked`, `refusals`, `records`, `damage_in_log`, `logged_commits`, `safe_snapshots` (each safe snapshot marker as `(Position, Snapshot)`: the commit position it covers and its snapshot), `appended`, `durable`, `checkpoint`, `dirty`, `walks`, `page`, `page_size`, `mapped_to`, `on_history` |
| `TxnProbe` | `next_xid`, `commit_log_damage` |

Controls, all on `Testing`:

| Group | Calls |
| --- | --- |
| Addressing | `heap`, `history`, `large`, `index`, `history_index` (an `Object` by qualified name), `mark_of`, `own_mark`, `owner`, `root`, `pages`, `owned`, `reachable`, `commit_log`, `ctors`, `key_schema` |
| Reading an object | `entries`, `all_entries`, `versions`, `read`, `headers`, `status`, `is_free`, `allocated` |
| Changing the engine | `plant(Plant)` |
| Durability | `flush`, `flush_log`, `flush_pages`, `checkpoint`, `recycle_log` |
| Holds and hooks | `hold_vacuum`, `hold_page`, `on_reindex_phase`, `take_xid` |
| Checks | `verify_holding`, `verify_pages`, `check_page`, `scrub_page`, `check_indexes`, `probe_heaps`, `refuse_named`, `backup`, `archive_status` |

`OpenOptions::testing` carries `Knobs`, the settings only a test gives
an open: `Knobs::without_background_vacuum()` starts no background
vacuum worker, `with_segment_size(bytes)` creates a file whose log
segments are `bytes` each, a positive multiple of the 512-byte log
block, or open fails with an I/O error of kind `InvalidInput`, and
`with_limits(limits)` promotes the serializer's read locks at `limits`
in place of the defaults. An open of an existing file keeps the segment
size it was created with.

## 2. Contract

Postconditions:

- Every observation and control returns a value built at the call: a
  set, a map, a count, an LSN, an id, a flag, a page's bytes, an entry,
  a version or a finding. None returns a reference into the engine, and
  no type of this module holds one past the borrow of the database or
  the pager it was made from. The guards `VacuumHold` and `PageHold`
  hold their lock until dropped and expose nothing of it.
- The observations of `Testing`, `TxnProbe` and `Probe`, except
  `Probe::page`, `damage_in_log`, `logged_commits` and `safe_snapshots`,
  read memory only and change neither the engine's state nor what a
  later call returns.
- `Probe::page`, and every call that reads an object, reads through the
  pager as any reader does: a refused page answers its `PageError`, and
  a read that meets damage records it as any read records it. None of
  them sets a mark.
- `damage_in_log`, `logged_commits` and `safe_snapshots` read the
  retained log as `verify` does: they pin the log while they read, list
  its directory and open its segments through the runtime, which
  records which segments are open and, under the simulator, takes fault
  draws a later operation would otherwise take. `safe_snapshots` first
  writes and forces every record appended so far. They return nothing when the
  segments cannot be listed or opened. `safe_snapshots` panics at a
  marker or a batch of the retained log that does not decode.
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

## 3. Controls

An `Object` names a structure by the id the catalog gives it: a
constructor's heap, its history heap or large-value store by `HeapId`,
an index or the history key index of a relation by `IndexId`, or one of
the engine's own structures (the commit log, the commit times tree, the
catalog, the counters, the marks page, the damage list). A test finds
one by qualified name and holds no reference into the engine.

`plant(Plant)` is the one way a test changes the engine's state beneath
its public calls; every variant of `Plant` is one such change. The
other controls only drive the engine (write, wait, hold, hook) or read
it, and say below which of them record damage or set marks.

### 3.1 Objects

- A call on an `Object` resolves its id against the engine's current
  catalog at the call. After a rebuild an `Index` names the rebuilt
  tree. An object the catalog no longer holds answers
  `Failed::Absent`; a structure with no listing of pages answers
  `Failed::Unlisted`.
- `mark_of` answers the mark the engine sets for damage to a page of the
  object, as the engine's own mapping of pages to marks gives it.
- `own_mark` answers the mark that names a heap or an index alone, as a
  `Mark` plant sets it and admission of a call checks it; a history heap
  or index answers its relation's heap's or key index's. A large-value
  store or an engine structure answers none.
- `owner`, `root`, `pages`, `owned`, `reachable`, `ctors`, `key_schema`,
  `commit_log`, `entries`, `all_entries`, `versions`, `read`, `headers`,
  `status`, `is_free` and `allocated` answer what the structure holds
  at the call, read through the pager.

Not promised: the order of `owned`, or of the pages `reachable` lists;
which pages of a structure a later write reuses.

### 3.2 Plants

- `Refuse { page, damage }` makes every later read of the page fail as
  `damage` says and records the damage; it sets no mark until a call
  maps the damage. `RefuseKeptStore` does that for each page of the
  store of kept findings.
- `Mark` and `Unmark` set and clear a mark durably.
- `Panic { site, message }` panics the engine once at `site` with
  `message`: `BackgroundRead` at the background worker's next read of a
  page, `KeptWrite` at the next write of kept findings after its first
  replacement.
- `AddEntry`, `AddRawEntry`, `DropEntry`, `MisorderLeaf` and `NextTid`
  change an index or a heap beneath every check of the write path. An
  entry change that finds nothing to change answers `Failed::Unchanged`.
- `Write`, `FlipOwner` and `Replace` change a page as one logged change;
  `Free` frees a page in the allocation bitmap whatever names it;
  `NamingSequences` leaves the catalog as a crash inside naming the
  counters page leaves it.
- `Orphan { kind, owner }` allocates a page no structure names, its
  contents zero, and answers `Planted::Page`.
- `Insert { heap, rows }` commits the rows as one transaction of the
  write path, with no index entry and no check, and answers
  `Planted::Tids`.
- `LoadMarks(page)` reads the marks from `page` as an open does and
  replaces the marks held in memory; an unreadable page answers its
  `PageError` and leaves them.
- `FailOn(error)` fails as the write path fails on meeting `error`,
  mapping the damage and setting the marks that sets, and answers
  `Planted::Refused`.

Not promised: that a plant leaves the file in a state a real fault
could reach beyond the one it names. A test reads what the engine does
about the plant, not that the plant was possible. Which thread or order
the engine meets a `Panic` plant in when two could reach its site.

### 3.3 Driving

- `flush` writes the log, then every changed page; `flush_log` writes
  the log; `flush_pages(max)` writes at most `max` changed pages whose
  changes are durable and answers how many; `checkpoint` takes a
  checkpoint; `recycle_log` removes the segments no recovery or archive
  needs and answers how many. None records damage or sets a mark.
- `hold_vacuum` holds the vacuum lock until its guard drops, holding off
  vacuum, the background worker's included, and every rebuild.
  `hold_page` holds the page's exclusive latch until its guard drops.
- `on_reindex_phase` calls its hook with the name of each phase of every
  later rebuild as the phase starts, on the rebuild's thread.
- `take_xid` begins a transaction and aborts it, writing nothing else;
  it answers the id it took.

Not promised: when a write-out the engine starts on its own lands
relative to these calls; the set of phase names `on_reindex_phase`
reports.

### 3.4 Checks

- `verify_holding` runs `verify` over the whole database with the checks
  that hold only with no writer, holding off vacuum for the run, and
  answers its rows. It records damage, sets marks and keeps findings as
  `verify` does. Precondition: no other call runs on the database
  meanwhile.
- `verify_pages` checks every page and the retained log; `check_page`
  checks one page against the retained log; `scrub_page` does what the
  background scrub does for one page. Each refuses from now on a page
  whose contents it finds lost, recording the damage, and sets no mark.
- `check_indexes` checks the indexes of the relation an index belongs
  to, at a snapshot taken before its callback runs, holding off vacuum
  across each index. `probe_heaps` probes a key index from its
  relation's heaps as a read does while that index is marked.
  `refuse_named` runs a read's check of the version an entry names
  before it refuses the index: a page the heap does not own is refused
  and its damage recorded, and a free slot answers an orphan entry.
  They record damage they meet and set no mark; the caller decides what
  a finding marks.
- `backup` copies the database to a target as a backup does, calling
  its visitor with each page as it is first read; it records damage it
  meets and sets no mark.
- `archive_status` answers the log archive's status as its last pass
  left it, `None` with no archive set. It reads memory only.

Not promised: the findings' order, or their text beyond their kind and
the page or object they name.

## 4. Ownership

The module owns the mapping from engine internals to its answers. When
storage, transactions or maintenance change how they keep a fact, this
module changes and no test does. The answers carry the engine's
vocabulary types (`Pid`, `Lsn`, `Damage`, `Quarantine`, `PageError`,
`Xid`, `LogFinding`, `vacuum::Charged`, `Entry`, `Version`, `Status`
and the findings of the checks) and this module's own types (`Records`,
`Object`, `Plant`, `Planted`, `Site`, `CommitLogPages`, `Failed`,
`Knobs` and the guards `VacuumHold` and `PageHold`), never a pager, a
heap, an index or the write path.

## 5. Harness

`adtb-simkit` holds what the simulator's test binaries share above
`adtb_sim`: the crash loop, the opens, the component world, byte
damage and the isolation workload over the engine
([runtime.md §6](runtime.md#6-isolation-histories)).

- A test defines no crash loop, world type or byte-damage helper of its
  own. Every crash loop is `crash_loop`, and every round of it runs the
  check. One test crashes a run more than once outside it: the damage
  search, whose crashes are steps of a generated sequence judged after
  the run.
- A test opens a database through `open` or a call of
  `Database::open_in` in its own body, and a world through `Storage`. A
  test may call `Database::open_in` and `Sim::crash_at` directly, to
  test the public open and the simulator.
- Local open helpers remain. `work/tools/structure-allowlist` counts
  them by name: functions taking parameters whose name begins or ends
  with an `open` form, closures bound to an `open` form, and `fn engine`
  where it opens the engine. A helper under another name is not
  counted. The size products left are counted the same way. The counts
  only fall.

```text
crash::crash_loop(&mut sim, seed, faults, Rounds::new(count, base, span),
                  workload: FnMut(&mut Round), recovered: FnMut(SimRuntime, &Ledger) -> T)
  -> Crashes<T>                                // T: Reopened (adtb_sim::Recovered)
Crashes::{rounds() -> &[Checked<T>], ledger(), allowed() -> (round, &Violation)..., crashed()}
Checked::{crashed(), allowed() -> &[Violation], reopened() -> &T}
Round::crash(body) -> bool; Round::crash_after_setup(setup, body: FnOnce(rt, T)) -> bool
Round::{seed, index, ledger, sim}
Reopened::{allows(&Violation), settle(&Ledger), last()}
open::{FILE, options(workers), open, opened, reopen}
fixture::Storage::{create(rt, &Options), reopen(rt, &Layout), layout,
                   commit_log, large, heap, index, tree, pager, split, parts, verify}
fixture::Parts::{new, verify}                // a world a test keeps in its own fields
damage::DataFile::{new(name, page_size), of(&Options), range, bytes, page,
                   page_mut, flip(files, pid, at), zero, replace, truncate, extend, cut}
```

Postconditions of `crash_loop`:

- Each round runs the workload, which crashes its process through
  `Round::crash` (under `faults`, at `crash_after(seed, round, base,
  span)` from the run's start), through `Round::crash_after_setup` (the
  same, counted from when `setup` returns, so no crash falls in
  `setup`), or ends its run another way through `Round::sim`. The loop
  then runs `recovered` as a process under no faults, with `faults`'s
  sector and atomic unit, and fails, naming the seed and round, unless
  it finishes.
- Every round runs `adtb_sim::check` of what `recovered` returns
  against the loop's ledger, which the workloads of every round record
  their commits in. A violation the reopened database's `allows` does
  not accept fails the loop, naming the seed and round; one it accepts
  is returned with the round, and `Crashes::allowed` lists them all. The check covers `verify`: it holds the
  reopened database's findings. No round skips the check.
- After a passing check the reopened database settles the commits in
  doubt, by default acknowledging those it reports committed and
  aborting the rest, so a later round's check holds them to that fate.
- The loop ends after `count` rounds, or after the first round whose
  reopened database is `last`.

Postconditions of the opens: `open` opens the database in a file of
the simulated device through `Database::open_in`, and `opened` fails
naming the file where `open` returns an error; `options(workers)`
creates the database when it does not exist. `reopen` opens a pager
over its file, recovering it from its log.

Postconditions of `Storage`:

- `create` makes the file and no structure. Each structure made after
  takes the next owner id of one counter, from 1, so no two structures
  of a world share an owner.
- `layout` answers where each structure lies, as a value that outlives
  the process. `reopen` of it answers a world holding the same
  structures, under the same owners and roots, in the order `create`'s
  world made them.
- `verify`, and `Parts::verify` over the same structures held
  elsewhere, run `verify` over every structure of the world, heap `i`
  as constructor `i` of one sum that every index ranges over, then
  `repair` and a second pass, and answer what either pass finds that
  is not mended.

Postconditions of `DataFile`: a page's bytes are the page-size run at
its id in the data file ([storage.md §1](storage.md#1-the-file));
`flip(files, pid, at)` inverts byte `at` of the page's body, the bytes
past its header ([storage.md §2.1](storage.md#21-page-header)), and
fails unless `at` lies inside the body. `truncate` keeps the first
`pages` pages, `extend` adds zero pages up to `pages`, `cut` keeps the
first bytes.

Not promised: which damage the engine detects, that a change yields a
given `Damage` on read, or a log segment's layout, which byte damage
does not address.
