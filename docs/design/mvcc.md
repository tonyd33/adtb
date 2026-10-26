# Versions and isolation

The transactions layer provides row versions, visibility, uniqueness
checks, serializable isolation and reclamation over the pages of
[storage.md](storage.md). Its consumers are the indexes of
[indexes.md](indexes.md), the executor of [execution.md](execution.md),
the maintenance of [maintenance.md](maintenance.md) and the replicas of
[replication.md](replication.md).

## 1. One heap per constructor

Each constructor of a relation-level sum has its own heap. A heap holds
rows of one schema only. A heap is keyed by (relation variable,
constructor id), whatever instantiation of a parametric sum the
relation variable holds.

Invariants:

- No constructor owns another's heap. A split or merge of constructors
  moves rows into the new constructors' heaps
  ([indexes.md §7](indexes.md#7-building-and-rebuilding)).
- A scan of a constructor relation reads its heap; a scan of the whole
  sum is the disjoint union of its heaps' scans, and an ordered scan of
  the sum merges ordered scans of each heap on the fly.

Each heap has a directory naming its pages, which is also its free
space map, and a visibility map of its own (§11). The catalog records
the heap id and the page id of the directory's root; the root names the
visibility map's root.

The directory is a tree of free space map pages owned by the heap
([storage.md §2.1](storage.md#21-page-header)). Each entry names a page
and holds a free space category: the page's free bytes times 256 over
the page size, at most 255. A directory page at level 0 lists heap
pages; one at level `l` lists directory pages at level `l - 1`. Pages
are numbered by position, in the order they were added, and each heap
page records its own position.

Invariants:

- The root's page id never changes. A full root moves its entries to a
  new directory page and lists that page alone, one level up.
- Every directory page but the last at its level is full, so the path
  to position `p` is the digits of `p` in base `fanout`, from the root.
  The last may be empty.
  `fanout`, the entries a directory page holds, is fixed when the heap
  is created.
- An entry with category `c` names a page with at least `c` times the
  page size over 256 bytes free. A level-0 entry of a page in the
  heap's newest row format has exactly its page's category, and
  changes only in the update that changes its page.
- An inner entry has at least the greatest category of its child, and
  is touched when an entry of its child is, except above a change not
  yet settled: a crash then may leave the entries above it promising
  too little. `verify` reports such an entry and `repair` settles the
  directory ([verify.md](verify.md)).
- A heap page stays in the directory, at its position, until vacuum
  finds it empty (§10.1) or the heap is dropped. A removed page leaves
  a hole at its position, which the next insert needing room fills
  with a new page at that position.

Postconditions:

- A scan of the heap reads its pages in position order and each page's
  versions in slot order.
- Each pager update a heap operation makes
  ([storage.md §2.3](storage.md#23-page-access)) latches at most five
  pages, transient reads included: at most two heap pages, at most one
  level-0 directory page, and a visibility map page, a bitmap page or
  further store pages within the bound. An insert, update, delete or
  retag changes its heap pages and the level-0 entry of the page that
  takes the version in one update. Before it, in updates of their own:
  the out-of-line values a page's conversion
  ([§12.4](#124-row-format-changes)) moves, written while no page is
  latched; the counts of every value the new version points at
  ([§12.5](#125-large-values)), at most three store pages each; the
  bit of each all-visible page it changes, cleared in the visibility
  map; and the pages it allocates. A crash between them leaves values
  unpointed at, counts too high or bits clear on all-visible pages,
  never the reverse.
- A change to a level-0 entry latches only the directory page holding
  it. One that raises the category or changes the touched mark then
  settles the entries above it, the lowest first, each in an update
  latching that entry's page and reading its child. Adding a page
  grows the root, adds each directory page the position needs and adds
  the page, each in an update of its own. A search for room that finds
  an inner entry promising more than its child holds sets it the same
  way.
- Any number of threads read and write one heap at once, and vacuum
  runs beside them. Each page an operation changes is held under its
  exclusive latch ([storage.md §2.4](storage.md#24-latches)) from the
  read that decides the change to the update that makes it. A reader
  decodes a version, out-of-line values included, under its page's
  shared latch.
- Latches are taken in one total order: heap pages in page id order,
  then large-value store pages in page id order, then directory pages
  by level, the highest first, and in page id order within a level,
  then visibility map pages in page id order. No thread holds a latch
  while it waits for one earlier in the order.

Rulings: storage/table-per-ctor, storage/sum-scan/merge-append, encoding/native-cost/union-index, engine/in-heap, layout/fill/fsm

### 1.1 Rows

A heap is created with its row type, a list of `heap::FieldType`: the
constructor's fields in declaration order, the sum's header fields
first. A field type is a scalar, a record of field types in
declaration order, or a sum, listed as its constructors in declaration
order, each a `heap::CtorType` with its constructor id and its fields,
the sum's header fields first.

A heap reads and writes a row as `Vec<adtb::key::Value>`, one value per
field of the row type, in its order. Consumers rely on this row type:

- A field of a declared record type is a `Value::Record` of its fields
  in declaration order.
- A field of a sum type is a `Value::Sum { ctor, fields }`, `ctor`
  indexing the sum's constructors in declaration order, as the row type
  lists them. A one-constructor sum's value is `ctor` 0.
- A newtype is its representation and a reference its target's key.

Preconditions: every row a caller writes has a value of each field's
type; the heap panics on a row that does not.

Postconditions:

- Every row a heap returns is in the heap's newest row format, whatever
  format its page holds ([§12.4](#124-row-format-changes)).
- A row read back equals the row written, after each newer format's
  conversion.
- How a row is laid out in its page, which fields go out of line and
  in what order fields are stored is the heap's
  ([§12.2](#122-row-body)); no consumer depends on it.

## 2. TIDs

A TID is 64 bits: a 48-bit page id and a 16-bit slot. In an index
entry it is stored big-endian. A TID names its page in the global page
space ([storage.md §1.1](storage.md#11-one-page-space)), so a TID read
from one heap's version may name a page of another heap of the sum.

A TID stays valid while the version it names is retained: in-page
compaction moves tuples without changing their TID.

In a version header a TID is one little-endian `u64`, the page id in
the upper 48 bits and the slot in the lower 16. Zero names no version,
since page 0 is a file header copy.

Rulings: mvcc/version-tid

## 3. Version header

Every version starts with a 24-byte header:

```text
0   u64  xmin    inserting transaction
8   u64  xmax    deleting or superseding transaction, 0 when none
16  u64  next    TID of the successor version, in this heap or, when moved, another heap of the sum
24       body, at a fixed offset for every row of the heap
```

Each version also has a `u16` of flags held in its slot, not in the
header ([§12.1](#121-page-arrangement)). A flag change is a page change
and is logged as one.

| Flag | Meaning |
| --- | --- |
| moved | The successor is in another heap of the sum; `next` names it (§6) |
| retagged-in | This version was inserted by a retag |
| hot-updated | The successor is a heap-only version on this page |
| heap-only | No index entry names this version |

The heap gives `xmin`, `xmax` and `next` to its callers as the raw
values stored, and decides no visibility. A writer that sets a
version's `next` sets its moved and hot-updated flags for that
successor, clearing any an aborted writer left; a delete sets `xmax`
and clears `next` and both flags.

There is no attribute count, header length or null bitmap: the heap's
schema and the page's format version fix the body layout, and no field
is NULL. There are no commit-status bits; readers consult the commit
log (§4). A transaction's command ids for its own versions live in
its local state.

Rulings: layout/tuple-header/lean

## 4. Transaction ids, the commit log and snapshots

### 4.1 Transaction ids

Transaction ids are 64 bits and never wrap, so versions are never
frozen. Ids are assigned in increasing order from 1; the value 0 is
never an id and is what `xmax` holds when no transaction deleted the
version. Constructor ids need no counter: they follow from the operator
log.

Ids are assigned from a reservation. Before the first id of a new
reservation is assigned, the transactions layer logs an xid allocation
record (`Pager::reserve_xids`) naming the first id past it, and forces
the log.

Postconditions:

- An id is never assigned twice, across any number of crashes, whether
  its transaction committed, aborted or was running at a crash.
- Assigning ids never exhausts them in practice: at one id per
  nanosecond the 64-bit space lasts over 500 years. Once the last id
  is spent, starting a read-write transaction is refused; snapshots
  and reads go on. The id `2^64 - 1` is never assigned.

### 4.2 The commit log

The commit log holds 2 bits per xid (in progress, committed, aborted)
in commit log pages of the file. Commit positions are not in the
commit log; for the time-travel window a separate map gives them
(§10.3). The commit log is found by its root page, whose page id the
catalog records.

Invariants:

- Every id below the reservation has a status; an id's status changes
  once, from in progress to committed or aborted.
- Readers never write commit log pages or heap pages to record a
  status.

A commit changes the commit log page to committed and then logs the
commit record; an abort does the same with aborted and the abort
record. A commit is durable once the log is durable past its commit
record. A transaction, read-only included, is acknowledged only once
the log is durable past its own commit record and past the commit
record of every commit its snapshot reads; `Transactions::read_upto`
gives the last of those for a snapshot.

A commit log page that fails verification quarantines the whole
database, since every relation's visibility reads the commit log. From
then on every transaction is refused with the corruption fault: no
transaction starts, takes a snapshot, commits or aborts, and status
lookups fail naming the damaged page. The database opens for `verify`
and repair only: the commit log's root and directories stay readable
for them. If the root or a directory page itself is damaged, the
commit log does not open.

Any number of threads share the transactions layer's commit log.
A status lookup waits for no commit or abort: only for a reservation
of more ids or a truncation, which change the set of status pages, and
for the latch of a status page another thread is changing.

Precondition on the pager: a page change and the commit or abort
record logged by the next call on the pager lie in one batch
([storage.md §5.3](storage.md#53-blocks-and-segments)), so a crash
keeps both or neither.

Recovery replays the commit log pages as any pages
([storage.md §7](storage.md#7-recovery)); it does not read the commit
and abort records. Opening the commit log after recovery then records
as aborted every reserved id still in progress: the transactions
running at the crash and the ids reserved but never assigned.

Postconditions, after a crash and recovery:

- An id reads as committed exactly when recovery accepted its commit
  record, or it lies below the truncation bound. An acknowledged commit
  therefore stays committed.
- Every other id below the reservation reads as aborted. No id below the
  reservation reads as in progress until new transactions start.

### 4.3 Snapshots

A snapshot is:

```text
snapshot { xmin: u64, xmax: u64, in_progress: [u64] }
```

`xmax` is the next id to be assigned when the snapshot is taken,
`in_progress` the ids then running, in order, and `xmin` the oldest of
them, or `xmax` when none runs. Every id below `xmin` had resolved when
the snapshot was taken.

Visibility rule: a version's writer `x` counts as committed for
snapshot `s` when `x < s.xmax`, `x` is not in `s.in_progress`, and the
commit log says committed. The in-progress test precedes the commit log
lookup. Readers never write heap pages to cache a writer's status.

### 4.4 Truncation

The commit log is truncated from below.

Preconditions for dropping the commit log prefix below a bound `b`:

- every xid below `b` has resolved;
- vacuum has removed from every heap every version written by an
  aborted xid below `b`;
- vacuum has cleared the `xmax` of every version whose deleter aborted
  below `b` (§10.1).

Postcondition: every xid below `b` reads as committed, and no
remaining version names an aborted xid below the bound. The bound is
recorded in the commit log's root page, in the same change that drops
the prefix; the commit log pages that held only ids below it are freed.

Rulings: mvcc/xid-clog, mvcc/xid-width/xid64, mvcc/status/clog-lookup

## 5. Version chains

Chains run oldest to newest: an update writes a new version and points
the old one's `next` at it; index entries name the version they were
inserted for (version TIDs). A reader wanting the newest version walks
forward; an older snapshot stops at the first version it can see.

An update is heap-only when it fits on the same page and changes no
field that is a key field of an index whose constructor set contains
this heap's constructor. Field values are compared bytewise, so a
value-level sum field that changes constructor is a change. A heap-only
update adds no index entries; any other update inserts a new entry
into every index whose set contains the constructor. Creating an index
over a constructor set handles broken heap-only chains in those heaps
only.

A version whose header holds `xmin` and `xmax` is visible to a
reader `r` at snapshot `s` when:

- `xmin` is `r`, or counts as committed for `s` (§4.3); and
- `xmax` is 0, or is neither `r` nor counts as committed for `s`.

A version is visible to the transaction that wrote it whatever the
command; which of its own versions a statement sees is decided by the
command ids in the transaction's local state. Visibility reads no flag
of §3: a moved version is invisible to newer snapshots by its `xmax`,
like any superseded version. A deleter that aborted leaves the version
visible until vacuum clears its `xmax`.

The heap walks a chain from a TID through each `next`, across heaps,
and returns the first version a caller's predicate accepts, with its
TID, the heap id of its page, its header and flags; or none, when the
chain ends first. A redirect line pointer
([§12.1](#121-page-arrangement)) is followed to the slot it names. A
slot holding no version ends the chain, as do a page that is not a heap
page and a version whose `xmin` is not the `xmax` of the version before
it: a version's `next` may name a successor vacuum freed, and whose
slot or page was taken again.

A `next` that names no successor written by the version's deleter is
allowed only when that deleter has resolved: it aborted and vacuum has
not yet cleared it, or it committed and the successor is dead under its
own heap's watermark while the version is not yet freed under its own,
as a retag across heaps with different watermarks leaves. A version
whose deleter is running always names a successor that deleter wrote,
and a version with no deleter names none.

An update names the key fields of every index whose constructor set
contains the heap's constructor. The heap decides whether the update is
heap-only and reports it; the caller inserts index entries for the new
version exactly when it is not.

Rulings: mvcc/o2n, mvcc/version-tid, mvcc/hot/per-heap

## 6. Retag

Retag keeps the row's key and moves the row atomically into another
constructor's heap. For the update below on the `EmailPass` row with
key `k`:

```adt
update i in identities where i.id == k
set match i
  | EmailPass e -> Google { user: e.user, gid: g }
  | Google o    -> o
  | Apple a     -> a
```

the writing transaction `T`:

1. Inserts the new version in the `Google` heap at `tid'` with
   `xmin = T` and the retagged-in flag.
2. Sets `xmax = T`, the moved flag and `next = tid'` on the old version
   in the `EmailPass` heap.
3. Inserts an entry `(k, Google, tid')` into every index whose
   constructor set contains `Google`. The old entry `(k, EmailPass,
   tid)` stays in every index containing `EmailPass` until vacuum.
4. Checks each new entry against range locks as an insert of a
   `Google` row (§8.2).

Postconditions:

- A retag is never heap-only.
- Each out-of-line value the new row shares with the old version keeps
  its pointer ([§12.5](#125-large-values)).
- Snapshots older than `T` find the old version through the old entry;
  newer snapshots find the new one.
- A uniqueness probe that finds the old entry skips it because its
  version's deleter is `T` itself.
- A walk of the chain to the newest version (vacuum, `verify`) follows
  `next` across heaps: page ids are global, so the successor's TID
  names its page in the other heap. A transaction that retags a row
  from `A` to `B` and back to `A` leaves each version's `next` naming
  exactly its own successor.

A transaction that finds the old version being updated, deleted or
retagged by `T` waits for `T`. If `T` commits, the waiter fails with a
serialization failure; if `T` aborts, the waiter proceeds on the old
version. A waiter never re-evaluates its statement on a successor.

Rulings: union-index/retag/move, engine/retag-chain/forward-pointer, engine/retag-snapshot/two-entries, mvcc/link/key-relookup, layout/succ/key-probe, mvcc/link/reinsert/in-flag

## 7. Uniqueness checks

An insert into a unique index probes its key range
([indexes.md §2](indexes.md#2-entries)). For each equal entry it
fetches the version from the heap the entry's constructor id names and
checks it against the latest committed state, not the inserter's
snapshot:

| Version found | Outcome |
| --- | --- |
| Live, committed, visible to the inserter's snapshot | Conflict |
| Live, committed after the inserter's snapshot | Serialization failure |
| Inserted by a transaction in progress | Wait for it; serialization failure if it commits, proceed if it aborts |
| Deleted by a transaction committed before the inserter's snapshot | Skipped |
| Deleted by a transaction committed after the inserter's snapshot | Serialization failure |
| Deleted by a transaction in progress, the version visible to the inserter's snapshot | Wait for it; serialization failure if it commits, conflict if it aborts |
| Dead to every snapshot, or deleted by the inserter itself | Skipped |
| Inserted by another write of the same statement | Conflict, reported at the statement boundary |

A wait ends when the awaited transaction commits or aborts; the
inserter then decides the same version again by the table, so a wait
on an inserter that commits ends in a serialization failure, and one on
a deleter that aborts in a conflict. Waits for a writer of a row (§6)
are decided the same way: a version whose `xmax` names a running
transaction waits for it, one whose deleter committed is a
serialization failure, one whose deleter aborted proceeds.

Preconditions:

- The probe and the insert of the inserter's own entry are one atomic
  step with respect to every other insert of the same key, so of two
  concurrent inserters of a key, one finds the other's entry.
- Nothing latched is held across a wait; a wait parks through the
  runtime ([runtime.md §1](runtime.md#1-time-threads-and-randomness)).

Postconditions:

- A wait that would close a cycle of transactions waiting for each
  other fails the waiter with a serialization failure instead.
- An inserter never waits for itself, and never fails or conflicts on a
  version it deleted.

Invariant: a key may hold several entries, at most one of them for a
live version. No other constructor's heap is locked.

The error a conflict reports is stated in
[../spec/semantics.md §6.6](../spec/semantics.md#66-statements).

Rulings: engine/wait-inserter

## 8. Serializable snapshot isolation

### 8.1 What the transactions layer guarantees

Every transaction is serializable; there is no weaker level
([../spec/transactions.md §1](../spec/transactions.md#1-serializability)).
The transactions layer provides it by serializable snapshot isolation
over the in-heap versions: readers read their snapshot and never block;
a writer that would create a dangerous structure of two consecutive
rw-antidependencies among concurrent transactions aborts one of them
with a serialization failure. A write that meets a row version
committed after the writer's snapshot (an update, delete or retag of a
row a later commit changed, or a wait on such a writer, §6) is a
serialization failure, never a key violation and never a re-evaluation
on the newer version.

A statement reads constructor relations through predicates. Its
denotation depends on `D(r.C)` only through sets `{ v ∈ D(r.C) | P(v) }`
for predicates `P` the transactions layer tracks; a read of `r.S` is a
predicate read of every `r.C` with `C ∈ S`. A write `T'` changes the
matches of a predicate read of `r.C` by `T` when a version `T'`
installs, or the one it replaces, is a row of `r.C` that satisfies `P`.

The dependency graph of a history has the committed transactions as
nodes and an edge `T → T'` when:

| Edge | Condition |
| --- | --- |
| write-write | `T'` installs the next version of an item after a version `T` installed |
| write-read | `T'` reads a version `T` installed, directly or through a predicate |
| read-write | `T'` installs the next version of an item, or changes the matches of a predicate, after the version or state `T` read |

Postcondition: every history the database produces has an acyclic
dependency graph over its committed transactions. A transaction that
would make the graph cyclic does not commit; it fails with a
serialization failure. Each test against a later commit
([../spec/semantics.md §8.3](../spec/semantics.md#83-tests-against-later-commits))
is a read-write edge whose cycle the guarantee forbids.

Consumers may rely on serializability for rules that would otherwise
need their own locks:

- A constraint check, including an at-least-one check, is a read of
  its transaction, so two concurrent deletes that together violate it
  form a dangerous structure and one aborts. Concurrent checks cannot
  jointly admit a violation
  ([../spec/constraints.md](../spec/constraints.md)). The checks of
  [maintenance.md](maintenance.md) need no locks of their own.
- A reference check reads its target, and a retag's recheck of
  referrers reads the referring range; a concurrent insert of a
  referrer typed by the old constructor and the retag conflict through
  those reads.

### 8.2 SIREAD locks

SIREAD locks record reads; they never block. A writer checks the locks
its writes cover and records a rw-antidependency for each.

| Lock | Taken by | Conflicts with |
| --- | --- | --- |
| Tuple `(heap, tid)` | reading a version | an update, delete or retag of that version |
| Index range `(index, lo, hi, ctors)` | an index scan or probe, over the gaps between the entries it saw | an insert of an entry in `[lo, hi]` whose constructor is in `ctors` |
| Heap `(heap)` | a sequential scan of a heap | any insert into that heap |
| Schema version `(v)` | every statement, for the schema version it runs at | the commit of a migration producing a later version |

A read whose outcome depends on a writer its snapshot does not count
as committed records a rw-antidependency from the reader to that
writer, unless the writer aborted: reading a version whose deleter is
such a writer, and missing a version whose inserter is one. The locks
above cover writes after the read; this covers writes before it.

Range locks are next-key ranges, not leaf pages. Each range lock
carries the constructor set the query read, as a set over the index's
constructor ids, so a query over `identities.(Google | Apple)` that
scans the key index does not conflict with an `EmailPass` insert into
the range:

```adt
from i in identities.OAuth where i.user == u select i.id
-- range lock on the index over user, ctors {Google, Apple}
```

A lock may be widened to a coarser one (a page, a heap, a whole
index); a widened lock keeps the union of the constructor sets.

A retag's new entries (§6) are checked against range locks as
inserts of rows of the new constructor, so a serializable scan of
`identities.Google` that saw no row with key `k` conflicts with a retag
of `k` into `Google`. Tuple locks on the old version conflict as for
any update; they are not copied to the new version.

Applying a migration writes the schema version, so a transaction whose
statements ran at version `n-1` and a migration producing version `n`
form a rw-antidependency, and a transaction that also writes is
serialized against the migration like any other writer.

A bulk load that builds an index over an empty constructor set
([indexes.md §7](indexes.md#7-building-and-rebuilding)) inserts no
entries one by one. Its commit is checked as an insert into every
constructor heap it fills: it conflicts with every heap lock on those
heaps and every range lock, on any index over them, whose constructor
set contains one of them. Its constraint checks take SIREAD locks as
any read does.

A split or merge of constructors replaces the tuple, page and range
locks held on the rewritten heaps and their index ranges by one heap
lock on every heap the operator writes, so later writes to those
constructors conflict with the old readers.

A probe of a hash index ([indexes.md §6.2](indexes.md#62-hash-indexes))
takes a range lock whose two bounds are the probed hash value, with the
probe's constructor set; an insert of a bucket entry is checked as an
insert of an entry whose key is its hash. No hash index is built yet.

Range bounds are encoded keys ([indexes.md §3](indexes.md#3-key-encoding)),
so a scan of the keys with prefix `p` locks from `p` up to the first
byte string past every extension of `p`, and an equality probe locks
`[k, k]`.

### 8.3 Deferrable read-only transactions

Reads on the primary run under SSI and never wait for a safe snapshot.
Only a transaction declared `read_only deferrable` waits for one on
the primary: a snapshot with no concurrent read-write transaction that
could still make it part of a dangerous structure. It then runs without
SIREAD locks and never aborts with a serialization failure. The primary
logs a marker for each safe snapshot it finds
([replication.md §2](replication.md#2-safe-snapshot-markers)).

A read-only snapshot is pending until every read-write transaction
running when it was taken has ended. It is unsafe once one of them
commits with a rw-antidependency on a transaction that committed before
the snapshot; it is safe when all have ended and none did so. A
deferrable transaction waits, parked through the runtime, until its
snapshot is decided, and takes a new snapshot when it is unsafe. Any
read-only transaction whose snapshot becomes safe drops its SIREAD
locks and records nothing more.

The marker is logged when the serializer finds a snapshot safe, at the
start of a read-only transaction with no concurrent read-write one or
when the last of them ends, and never for an unsafe snapshot. It covers
every commit at or before the commit position of the latest commit the
snapshot reads (§4.2), or, when it reads only commits from before the
commit log was opened, the position the log was durable to then. The
snapshot is encoded as `xmin` and `xmax`, then each in-progress id in
order, each a little-endian `u64`.

### 8.4 Dangerous structures

Two transactions are concurrent when neither committed before the
other's snapshot was taken. A rw-antidependency `T → T'` between
concurrent transactions is recorded at whichever comes second:

- `T` reads a version whose inserter `T'` its snapshot misses, or whose
  deleter `T'` it does not count as committed, and `T'` has not
  aborted;
- `T'` writes what a SIREAD lock of `T` covers.

A dangerous structure is `T1 → T2 → T3` with `T3` the first of the
three to commit (`T1` may be `T3`); if `T1` is read-only, `T3` also
committed before `T1`'s snapshot. When a new rw-antidependency or a
commit would complete one, the transactions layer aborts one
transaction of it with a serialization failure: the pivot `T2` when it
has not committed and is not the caller, else the caller.

Preconditions on the transactions layer's callers:

- A read takes its SIREAD lock before it reads the page, or in one
  atomic step with the read; a write checks the SIREAD locks after its
  change is in the page, or in one atomic step with it.
- Taking a snapshot is atomic with registering the transaction, and
  recording a commit in the commit log is atomic with recording it
  committed, so a transaction counts as committed before a snapshot
  exactly when the snapshot sees it.

Postconditions:

- SIREAD locks and rw-antidependencies of a committed transaction are
  kept until no running transaction is concurrent with it, then
  dropped; an aborted transaction's are dropped at once.
- A transaction chosen to abort fails its next read, write or commit
  with a serialization failure.
- A serialization failure names its cause (a row or key committed
  after the snapshot, a dangerous structure, or a cycle of waits) and
  the xids of the other transactions in the conflict, in order; a
  read-only transaction has no xid and is not named.

Rulings: engine/serializable-only, engine/ssi, engine/write-skew/by-isolation, engine/retag-fk/by-isolation, mvcc/next-key, mvcc/ctor-tagged, mvcc/retag-phantom/as-insert, mvcc/split-locks/promote-heaps, engine/retry/safe-snapshot

## 9. The watermark

Reclamation is bounded by one oldest-snapshot watermark, computed per
heap. Vacuum may remove a version only when it is dead to every
snapshot the heap's watermark admits.

A transaction whose constructor relations are known (a registered
query or transaction, whose plan names them) publishes its snapshot
only to the heaps it can read: the heaps its plan reads, plus every
heap read by the checks and delete actions of the relations it may
write. The set is recomputed when the schema changes. An interactive
transaction, whose reads are unknown, holds back every heap. A report
over `invoices` does not hold back vacuum of `identities`. The
watermark of a heap also includes the oldest snapshot replicas report
for it ([replication.md §4](replication.md#4-vacuum-feedback)).
Versions kept for the time-travel window are handled by §10.3.

The watermark `w(h)` of heap `h` is the least of the next id to be
assigned, the `xmin` of every live snapshot published to `h`, and every
id a replica last reported for `h`.

Invariant: `w(h)` is never past the `xmin` of a live snapshot published
to `h`, nor past a replica's report for `h`.

Postcondition: a version of `h` whose `xmax` names a transaction that
committed, with `xmax < w(h)`, is dead to every snapshot published to
`h`, live or taken later; a version whose `xmin` names a transaction
that aborted is dead to every snapshot.

Rulings: mvcc/watermark, mvcc/long-reader/ctor-scoped

## 10. Reclamation

### 10.1 Vacuum

Vacuum runs in the background per sum. Its contract with the indexes
and with the commit log:

- It removes from every index of the sum every entry whose version is
  dead to every snapshot the heap's watermark admits, or whose
  inserter aborted, before it frees that version's slot.
- It frees dead slots, updates the free space map, and sets all-visible
  bits where every remaining version is visible to every snapshot.
- It clears `xmax` of versions whose deleter aborted, with their `next`
  and their moved and hot-updated flags.
- It removes from the heap's directory every page it leaves empty, and
  frees the page.
- It frees every out-of-line value of the sum's large-value store that
  no version of the sum points at any more (§12.5), and removes the
  store's empty pages. It reads only the store pages written or
  released since the sweep that last settled them.

A version is dead to every snapshot the watermark `w(h)` admits when
its inserter aborted, or its deleter committed below `w(h)`. A version
is visible to every snapshot, live or taken later, when its inserter
committed below `w(h)` and it has no deleter; it stays so until it is
next written, and every write to a heap page clears the page's
all-visible bit in the same pager update.

Postconditions of a vacuum of a sum begun when the oldest running
transaction was `x`, or when none ran and `x` was the next id:

- no index entry of the sum names a slot vacuum freed;
- no version of the sum names an aborted transaction below `x`, in
  `xmin` or in `xmax`, so the commit log may be truncated below `x`
  once every other sum has been vacuumed as well (§4.4);
- a live snapshot reads what it read before.

Writers, readers and on-access pruning of the sum run beside a vacuum.
Each heap is judged for the whole of one vacuum by the least of its
watermark and the oldest transaction running when the vacuum first
judges it: every transaction below that bound had resolved, so a
version found dead stays dead from the scan that collects it to the
prune that frees it, whatever snapshots are taken meanwhile.

Precondition: no other vacuum runs of the sum, or of a sum sharing its
large-value store.

Vacuum's progress, triggers and budget are exposed as described in
[../spec/catalog.md §11.5](../spec/catalog.md#115-vacuum).

### 10.2 Pruning

Between passes, any access holding a heap page exclusively prunes
versions on it that are dead to every snapshot: heap-only versions are
freed, and versions still named by index entries leave a dead slot for
vacuum's index pass. Pruning is logged. A writer finding a page
without room for a new version prunes it before looking further
(`vacuum::insert`, `vacuum::update`).

A heap-only chain whose leading versions are dead keeps the slot index
entries name as a redirect to its first live version and frees the
others. A fixed-slot page holds no dead slot or redirect: it frees a
chain only once every version of it is dead and its entries are gone,
and keeps a chain with a live version whole. Pruning never sets an
all-visible bit; a page it changes loses its bit.

### 10.3 History heaps and reads of the past

With a time-travel window
([../spec/derived.md §6](../spec/derived.md#6-reading-the-past)), a
version deleted within the window and dead to every running snapshot
is moved by vacuum into a history heap of its constructor instead of
being freed, so current heaps and indexes do not hold history. Each
history heap has a key index whose entries are `(key, commit position,
tid)`, the commit position being that of the version's inserter.

A read `at commit p` reads at the latest safe snapshot marker at or
before `p`, a function of `p` listed in `sys::safe_snapshots`
([../spec/catalog.md §1.5](../spec/catalog.md#15-commit-times-and-safe-snapshots)),
and takes no SIREAD locks. It reads the current and history heaps of
the constructors it reads, probing the history key index by key where
the current read would probe the key index, and counts a version's
writer as committed when the writer's commit position is at or before
the marker. Versions past the window are removed from history heaps.

For every xid committed within the window a commit position map beside
the commit log gives its commit position, and the commit records'
wall-clock times map a time to a position.

### 10.4 Removing a constructor

Removing a constructor applies only to a constructor with no live
row, but older snapshots, the time-travel window and index entries may
still name its versions. The remove keeps the heap as history; vacuum
removes its dead versions and their index entries in its batched pass.
The heap is dropped and its pages freed once it holds no version any
snapshot or the time-travel retention window can see. Older snapshots
and `at commit` reads keep seeing the constructor's rows until then.

Rulings: mvcc/background, mvcc/on-access, union-index/vacuum/batched, derived/time-travel/retention/history-heap, union-index/drop-ctor/after-vacuum

## 11. Index-only scans

Each heap has a visibility map with one all-visible bit per page, set
by vacuum and cleared by any write to the page. An index-only scan may
skip fetching a version when the all-visible bit of its page is set in
the visibility map of the heap of the entry's constructor in the
index's relation variable.

Invariants:

- A page's bit is set only while every version on it is visible to
  every snapshot published to its heap, live or taken later, and no
  slot on it is dead.
- A page's bit is set only while the all-visible flag of its page
  header is set ([storage.md §2.1](storage.md#21-page-header)), and
  the flag only while every version on the page is visible to every
  snapshot. Setting the bit sets both in one pager update; clearing
  it clears the bit in an update of its own before the one that
  clears the flag and changes the page.
- No bit is set for a page that is not a page of the heap.

The map is keyed by page id, so a lookup reads no heap page.

Rulings: mvcc/vm-per-heap, btree/ios/vm-per-heap

## 12. Notes for the implementation

Non-binding.

### 12.1 Page arrangement

The heap's schema picks the page arrangement. A constructor whose
fields all have fixed width (including value-level sums stored at
maximum payload, §12.3) has fixed-slot pages; any other constructor has
slotted pages. A page keeps its arrangement when it is converted to a
newer row format (§12.4), except that a fixed-slot page converted to a
format with variable-width fields becomes slotted.

Both arrangements start with the page's position in its heap's
directory (§1).

Fixed-slot page:

```text
page header (24 B)
u32 position
u16 width w                      24 + body width, padded to 8
u16 n                            the most slots of width w the page holds
u64 used_slots[ceil(n / 64)]     bit i set: slot i holds a version
u16 slot_flags[n]                the version flags of slot i (§3)
slot[0] slot[1] ... slot[n-1]    from the next multiple of 8, slot i at a fixed offset
```

A TID's slot number is the array index. Freeing a slot clears its bit,
its flags and its bytes; no compaction occurs in a fixed-slot page.

Slotted page:

```text
page header (24 B)
u32 position
u32 upper                        offset of the lowest tuple byte
u16 slot_count
6 B reserved
line pointer[0] ... line pointer[slot_count-1]   8 B each: u16 offset, u16 length, u8 state, u8 reserved, u16 flags
free space
tuples, packed from the end of the page, each at a multiple of 8
```

Offsets and lengths are 16 bits, which covers every page size up to
64 KiB; a tuple is at most a page minus 48 bytes, rounded down to 8.
Line pointer states are unused (0), normal (1), dead (2: pruned, still
named by index entries) and redirect (3: its offset names the slot of
the next live version of a heap-only chain on the page). Tuples move
during in-page compaction without changing their TID. The line
pointer's `flags` are the version flags of §3. Emptying a slot drops
the unused line pointers at the end of the page, so `slot_count` is
one past the last slot in use.

A page's free space for the directory is, in a fixed-slot page, its
free slots times `w`; in a slotted page, the bytes no line pointer or
tuple takes, less one line pointer.

Directory page (§1), of kind free space map and owned by the heap:

```text
page header (24 B)
u16 level
u16 count
u16 capacity                     the directory's fanout, the same on each of its pages
u16 reserved
u64 vm                           the visibility map's root in the directory's root, 0 elsewhere
u64 entry[count]                 page id in bits 0 to 47, free space category in bits 48 to 55
```

A level-0 entry naming page id 0 is a hole: its page was removed. A
hole has category 255, so a search for room finds it; filling it
allocates a page at its position and names it in the entry. A
large-value store's directory is built the same way, with no
visibility map.

Rulings: layout/slots/by-schema

### 12.2 Row body

The row type a heap returns is stated in §1.1; this is its byte layout.

Fields are placed by the engine, not in declaration order: fixed-width
fields are ordered by decreasing alignment to remove padding, ties in
declaration order, then,
when the row has variable-width fields, a table of `u16` end offsets
for them from the next even offset, then a bitmap with one bit per
variable-width field, set when it is out of line (§12.5), then the
variable-width bytes. End offsets count from the start of the body.
The catalog records, per heap format version, each field id's offset,
so a rename or a move of a field changes no stored byte.

| Base type | Fixed part |
| --- | --- |
| `I8`, `I16`, `I32`, `I64` | little-endian, aligned to its width |
| `F32`, `F64` | the bits of the canonical value, aligned to its width |
| `Decimal s` | the coefficient as an `i128`, 16 B, aligned to 16 |
| `Instant`, `Time`, `DateTime`, `Duration` | `i64` microseconds |
| `Date` | `i32` days |
| `Period` | `i32` months, then `i32` days, aligned to 4 |
| `Text`, `Bytes`, `SortKey`, `ZonedDateTime` | variable width |

A record field whose fields all have fixed width is placed in the
fixed part by the same rule, padded to its alignment; any other record
field is variable width.

```adt
type Order = { paid: Bool, id: I64, qty: I16, total: Decimal 2 }
```

```text
body:  0 total (16)  16 id (8)  24 qty (2)  26 paid (1)     27 B, padded to 32
```

For a variable field `j`, the bytes are `[end[j-1], end[j])`, with
`end[-1]` the start of the variable area. A `Text`, `Bytes` or
`SortKey` field is its bytes. Any other variable-width field is its
value in the nested encoding: fields in declaration order, fixed-width
base types as in the fixed part, `Text`, `Bytes`, `SortKey` and the
zone of a `ZonedDateTime` (after its `i64` local time) prefixed by
their length as an unsigned LEB128 varint, and a sum as its tag (none
for one constructor), then its constructor's fields.

Rulings: layout/field-order/packed, layout/var-fields/offset-table

### 12.3 Value-level sums in rows

A field whose type is a sum is stored in the layout its shape allows:

| Shape | Layout |
| --- | --- |
| Every constructor fixed width and the largest payload within 64 bytes | tag, then a payload area the size of the largest constructor's payload, at the next multiple of the payloads' alignment; the whole padded to the larger of the tag width and that alignment |
| Otherwise | tag in the fixed part; the payload as a variable-width field |

A payload is its constructor's fields placed as a record's are.

The stored tag is the constructor id, the same number the wire
carries. Where the tag lies in the variable-width part of a row it is
an unsigned LEB128 varint. Where it lies in the fixed part (a fixed-slot
page, or the fixed part of any row) it is a little-endian unsigned
integer of 1, 2 or 4 bytes, the smallest that holds the largest
constructor id of the type when the page format is created; an id
outgrowing it gives the heap a new format version. Nullary constructors
store only the tag. A one-constructor sum (a record) stores no tag.

```adt
type Status = Draft | Sent { at: Instant } | Paid { at: Instant }
```

```text
status field, fixed: u16 tag | pad 6 | i64 at       16 B; Draft leaves 8 B unused
```

Adding a constructor whose payload is larger, or whose id outgrows
the format's tag width, gives the heap a new format version; rows are
rewritten lazily as their pages are written (§12.4).

Rulings: layout/vsum/by-shape, layout/vsum-tag/varint, layout/vsum/widen/rewrite, layout/tag-value/catalog-id

### 12.4 Row format changes

Each page holds rows of one format version. A schema operator that
changes a constructor's fields or widens a value-level sum gives the
heap a new format version, with a function that makes a row of the new
format from a row of the one before. Pages are converted lazily. Reads
decode a page by its own version and pass the row through each newer
format's function.

A change to a version's header or flags leaves the page's format as it
is. An insert into a page of an older format first converts every
version on it to the newest format, each in its own slot. A page whose
converted versions would not fit keeps its rows and format and gets
category 0 in the directory, so it takes no insert until vacuum frees
space on it. No conversion moves a row: TIDs and index entries are
unchanged.

Per-constructor row heaps are the only storage format; analytic speed
comes from execution.

Rulings: layout/heap-format/page-version, project/nongoals/columnar-store

### 12.5 Large values

A value too large for a page goes out of line to a large-value store
belonging to the sum, shared by all its constructor heaps; the row
holds a fixed-size pointer to it. A retag (§6) moves the row
between heaps and keeps its pointers, so no out-of-line value is
copied. Vacuum of the sum frees chunks no live or retained version
references.

While a version exceeds a quarter of a page, its largest inline
variable-width field goes out of line. A field of an update's or a
retag's new row equal to a value the old version points at keeps that
pointer, whatever its size. A row still longer than the largest
version a page holds is refused.

The pointer is 16 bytes: `u64` length, `u64` TID of the first chunk.
The store's pages are slotted heap pages owned by the store's id,
taken from the heap id space, with a directory of their own built as a
heap's (§1). Each tuple is one chunk:

```text
0   u64  TID of the next chunk, 0 for the last
8   u64  the transaction that wrote the chunk
16  u32  count: in a head, the versions that point at the value
20  u32  role: 0 head, 1 tail
24       bytes
```

A value's first chunk is its head; every later chunk is a tail named
by exactly one chunk's `next`. Chunks are written last first, each in
its own pager update that also makes the chunk written before it a
tail, before the version that points at them; each is written as a
head of count 0 on a page its update marks touched. The update that
writes a version counts it once more in the head of every value it
points at, so a value shared by an update or a retag counts each
version. A page's conversion counts the values it moves out of line in
updates of their own before the update that writes the converted
page. Pruning a version counts it off afterwards, in an update of
its own; a head left at count 0 marks its page touched.

The directory's level-0 entries carry the touched mark in bit 56, and
an inner entry carries it when any entry of its child does (§1), so
the sweep descends only into touched subtrees. The sweep settles the
store's directory first, then frees each head of
count 0 on a touched page whose writer has resolved, one chunk per
update, making the next chunk a head of count 0 as it goes, and
unmarks a page left with no head of count 0. A crash anywhere leaves a
head of count 0 on a touched page, or a count too high: the latter
`verify` reports and `repair` mends ([verify.md](verify.md)).

Rulings: layout/large/toast, layout/large/retag/per-sum

### 12.6 Vacuum phases

Vacuum runs per sum in phases:

1. Scan each constructor heap of the sum, skipping all-visible pages,
   and collect the TIDs of versions dead to every snapshot the heap's
   watermark admits, and of versions whose inserter aborted.
2. Make one pass over each index of the sum, deleting every entry whose
   `(ctor, tid)` is in the collected set.
3. Free the dead slots, update the free space map, set all-visible bits
   where every remaining version is visible to every snapshot, and
   clear `xmax` of versions whose deleter aborted. Versions kept for
   the time-travel window move to history heaps here (§10.3).

The dead set is a radix tree keyed by heap id and page, each
leaf a bitmap of dead slots; a sum's heaps are disjoint subtrees. When
the set reaches its memory budget, phases 2 and 3 run and phase 1
resumes.

Phase 1 collects, per page, the slots index entries name whose chain
on the page holds no live version: dead slots, versions not heap-only
and redirects. Every page it reads is pruned in phase 3, whether or not
it holds dead slots. Phase 3 judges each version again against the
watermark, frees the collected slots and every dead heap-only version
no remaining version or redirect names, and sets the page's bit when
every version left is visible to every snapshot. A page left empty
leaves the directory, freed in the same update. After the last round a
fourth phase sweeps the large-value store (§12.5).

Progress is a row per heap of a running vacuum (`vacuum::Progress`):
the phase, the fraction of the heap's pages scanned, and the dead
versions collected and not yet removed.

The visibility map is a radix tree of visibility map pages owned by the
heap, its root page fixed:

```text
visibility map page
  page header (24 B)
  u16 level
  6 B reserved
  level 0: one bit per page id, span = (page size - 32) * 8 ids from a multiple of span
  level l: u64 child[fanout], fanout = (page size - 32) / 8, 0 for none
```

A root at level `l` covers page ids below `span * fanout^l`; a page id
past it grows the root as the directory's does, moving the root's
contents to a new page that becomes its first child. A leaf and the
inner pages above it are added when a bit under them is first set. The
heap page header's flag bit 0 is the page's all-visible flag, so a
writer touches the map only when it finds the flag set.

Rulings: btree/dead-set/radix

### 12.7 Lock promotion

Under memory pressure tuple locks promote to page locks and then to a
heap lock, and range locks promote to an index lock; promotion keeps
the union of the constructor sets.

Pressure is counted per transaction: more tuple locks on one page than
a threshold become a page lock, more page locks on one heap a heap
lock, more range locks on one index an index lock whose constructor
set is the union of theirs. A lock already covered by a coarser one the
transaction holds is not taken. Beyond a budget on the locks of every
transaction together, the largest holder's locks promote one level:
all its tuple locks to page locks, else its page locks to heap locks,
else its range locks to index locks, until the total is within the
budget or nothing is left to promote. A write checks the tuple, page
and heap locks that cover the version it writes.

### 12.8 Index-only scans

An index-only scan reads `vm[heap][page(tid)]` for each entry and
fetches the version only when the bit is clear. A query that needs only
key fields and the constructor, such as a match whose arms read no
other field, is answered from the index. A heap whose constructor is
rarely written stays all-visible while other constructors of the sum
change.

### 12.9 Reads that record nothing

A read on a safe snapshot takes no SIREAD lock and records no
rw-antidependency: no read-write transaction can make a safe snapshot
unsafe. A past read (`at commit p`, §10.3) reads at a safe snapshot
marker, and a deferrable transaction or a read-only one whose snapshot
became safe (§8.3) reads at a safe snapshot, so none of them records
anything.

A change to a `count` or `sum` in a stored view
([../spec/derived.md §2.4](../spec/derived.md#24-concurrent-maintenance)) is an increment that
records no read.

### 12.10 Sequence generators

A `sequence` key's generator is a per-relation counter, and an insert
that does not supply a generated field gets the counter's next value.
An explicitly supplied value for a `sequence` key advances the
generator past it: after the change set commits, the generator's state
is at least one more than the largest value supplied. The advance is
not undone by a later abort.

Counters are reserved durably, as transaction ids are (§4.1). Each
counter's reservation is the first value past a block it may draw
from, held on a counter page whose page id the catalog records.
Before a counter draws a value at or past its reservation, the write
path logs the page change that extends the reservation and forces
the log. An advance past the reservation extends it the same way
before the statement's change set is checked. After a crash, every
counter resumes at its reservation.

Postconditions:

- A drawn value is never drawn again, across any number of crashes,
  whether the transaction that drew it committed, aborted or was
  running at a crash.
- After the change set commits, the counter stays past every value it
  supplied, after any crash.

The `load` values of rejected rows come from one more counter of the
same page, with the same guarantees.
