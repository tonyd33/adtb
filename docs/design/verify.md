# Structural verification

`adtb::verify` checks a whole database file for structural damage: the
pages and the log as the pager checks them, every heap, large-value
store, B-link tree and commit log, the routing from index entries to
heap slots, and the allocation bitmap against what refers to each page.
It runs only where no writer can exist. The `verify` command
([../spec/catalog.md §11.1](../spec/catalog.md#111-verification)) runs
its three checks while writers run, as section 4 states, and adds this
walk only on a file it holds.

## 1. The inventory

Until the catalog exists, the caller names the structures of the file
in an inventory:

```text
Inventory {
  commit_log: Option<&CommitLog>,
  sums:  [Sum { heaps: [(CtorId, &Heap)], indexes: [&Index] }],
  trees: [BTree],      // B-link trees outside any index
  pages: [Pid],        // pages the caller refers to outside every structure listed
}
```

A sum's large-value store is the one its heaps name; a store shared by
several sums is checked once.

Preconditions:

- Every page allocated in the file belongs to exactly one structure of
  the inventory, or is listed in `pages`. The structures own pages as
  follows: a heap its directory pages, its heap pages and its
  visibility map pages ([mvcc.md §1](mvcc.md#1-one-heap-per-constructor),
  [§11](mvcc.md#11-index-only-scans)); a large-value store its
  directory pages and chunk pages
  ([mvcc.md §12.5](mvcc.md#125-large-values)); a B-link tree every
  node reachable from its root by downlinks and right links
  ([indexes.md §8.1](indexes.md#81-b-link-trees)); the commit log its
  root, directory and status pages
  ([mvcc.md §4.2](mvcc.md#42-the-commit-log)).
- No write runs while `verify_with` or `repair` runs.

When the catalog exists it builds the inventory from its own records;
the catalog's pages are then one more structure.

A database quarantined whole may have no catalog to build an inventory
from. The `verify` command then runs no walk of this file
([storage.md §8](storage.md#8-quarantine)).

## 2. Findings

`verify(pager, inventory)` returns a report, and
`verify_with(pager, inventory, depth)` returns one to a depth:
`Depth::FULL` is `verify`; `Depth::STRUCTURE` leaves out the pager's
check of every page and of the whole retained log, so it reports no
`Unlogged` page and no log finding, returns no commits, and verifies
only the pages the structures read, as every read does.

```text
Report {
  findings: [Finding { pid, problem }],   // sorted, each once
  log: [LogFinding],                      // the pager's log findings (storage.md §7)
  commits: [xid],                         // every commit record in the retained log
  complete: bool,
}
```

| Problem | Meaning |
| --- | --- |
| `Damaged(damage)` | the pager refuses the page ([storage.md §3](storage.md#3-corruption-detection)) |
| `Unlogged` | a valid page no record of a log retained from its start wrote |
| `Structure(what)` | a structure's own check refuses the page; `what` names the structure and the fault |
| `Shared(a, b)` | structures `a` and `b` both refer to the page |
| `Unallocated` | a structure refers to the page, and the bitmap marks it free |
| `Leaked` | the bitmap marks the page allocated, and nothing in the inventory refers to it |
| `Overcounted { slot, count, versions }` | the head of an out-of-line value in `slot` counts `count` versions and only `versions` point at it |
| `Unsettled(what)` | a directory page of a heap or a large-value store has an inner entry promising less than its child holds ([mvcc.md §1](mvcc.md#1-one-heap-per-constructor)); `what` names the structure and the entry |

Besides each structure's own invariants, the checks cover:

- every value of a large-value store
  ([mvcc.md §12.5](mvcc.md#125-large-values)): its head counts at
  least the versions that point at it, a head of count 0 lies on a
  touched page, and every tail is named by exactly one chunk;
- every version's `next` that is not a heap-only successor on its page
  ([mvcc.md §5](mvcc.md#5-version-chains)): a version with no deleter
  names no successor, and one whose deleter the commit log says is
  running names a version that deleter wrote, when the inventory names
  the commit log;
- the commit log's status pages, which cover every id from its
  truncation bound to its reservation.

Postconditions:

- Every finding names the page that is wrong. An index entry naming a
  slot that holds no version, dead slot or redirect is a finding of
  the heap page the entry names.
- `complete` holds when every structure was read whole. A structure
  that cannot be read whole (a damaged page, a tree whose links are
  broken) leaves `complete` false, and no page is then reported
  `Leaked`, since the pages that structure owns are not known.
- A report with no findings and no log findings means the file is
  structurally sound for the inventory given.

## 3. Repair

`repair(pager, inventory, report)` frees every page the report found
`Leaked`, sets every head it found `Overcounted` to the number of
versions that point at it, settles every directory it found
`Unsettled`, and returns how many findings it mended.

Preconditions: the report is complete and was made against `inventory`
with no write since. `repair` panics on an incomplete report.

Postcondition: a `verify` right after reports no `Leaked` page, no
`Overcounted` head and no `Unsettled` directory page.

A crash between pruning a version and counting it off its values
leaves a head counting too many versions
([mvcc.md §12.5](mvcc.md#125-large-values)), and a crash during a
change to a directory may leave the entries above it unsettled. After every crash and
recovery, `verify` finds nothing that `repair` does not mend, and after
`repair` nothing at all.

## 4. A run under writers

`verify` and `verify r` take a read-only transaction that waits for a
safe snapshot, since a constraint read at another snapshot may see a
state no serial order holds. Neither runs in an open transaction or in
text that writes: the refusal is `command::outside_transaction`, shared
with `vacuum` and `reindex`, so a run never waits on its own caller.
Two runs may overlap, each with its own `sys::jobs` row and run number;
the lock on findings is held only for the number and the replacement.

Postcondition: with nothing damaged, a run reports nothing whatever
writers do meanwhile. Each check meets it as follows.

| Check | Runs | Why a problem it reports is not a writer's |
| --- | --- | --- |
| Page checksum and log position | `Pager::verify_live` | A page whose LSN is at or past the log's end at the start was written since and is passed over. A page that looks wrong is read again from the file under its exclusive latch, which no write-out or eviction holds with it. A log finding at or past that end is dropped |
| Each index's order, fences and levels | `BTree::check_live` | One latched read shows a page's own invariants. A fact about two pages counts only when the page linking to the other kept its LSN across the read of the other, so both held at one moment. A page without a downlink is rechecked against the page above it the same way. A pair a writer keeps changing is passed over after 16 reads and the walk goes on from the page reached, if it is a live page of that level |
| Row version to entry (`Missing`) | at the snapshot | A version the snapshot sees committed after its entries were written, and the entry names a root of its heap-only chain, read in one latch |
| Each link between versions on a heap page (`Structure`) | latest | One latched read shows both ends of a link. A writer changes a link and its successor under one exclusive latch. A hot-updated version names a successor its deleter wrote, and no two versions name one successor |
| Entry to row version (`Orphan`) | latest | The run holds off vacuum across each index. Only vacuum removes an entry or frees a slot an entry named, and a writer writes the version before the entry, so an entry read still names its version. A dead slot passes |
| Each admitted constraint (`Violation`) | at the snapshot | The snapshot is safe. A constraint reading a transition is not run. A constraint reading a damaged page reports that page, whatever the scope. One over the query memory is a `Structure` finding and the run goes on |

An index with a fault of its own structure gets no cross-reference in
that run, since what its walk returns is not what it holds.

`Leaked`, `Unallocated`, `Shared`, `Overcounted`, `Unsettled`, every
`Structure` problem of a heap or large-value store but the link check above,
and the version-chain checks cannot be made exact under writers, and the specification does not list
them. They run only in `adtb verify FILE`, which holds the file, by
`verify_with(pager, inventory, Depth::FILE)`. `Unsettled` and
`Overcounted` are never findings: a crash may leave them on a sound file
and section 3 mends them. `Leaked` is a finding: only a migration, a rebuild of an index or a
file's creation allocates a page apart from what names it, and open
frees what a crash of one leaves. Every other problem becomes a `Structure` finding, except `Damaged`, which becomes
`Checksum` or `StalePage`. `Unlogged` and log findings become
`Structure`.

`verify r` filters page findings to the pages of `r`'s heaps, their
large-value store and `r`'s indexes, and reports no log finding.

A running `verify` has a row in `sys::jobs`, removed when it ends, by a
panic included. A failure to keep the findings fails no run: the run
returns them with a `Structure` row saying they were not kept, and holds
them in memory until the next run over the scope or a reopen.
On a database quarantined whole the run keeps nothing in the file, and its
`Structure` row naming the mark says so
([storage.md §8](storage.md#8-quarantine)).

## 5. Kept rows

The rows of a volatile relation kept in the file are rows of constructor
14 of the roots heap, each `{ relation, group, ctor, format, row }`:
the relation's catalog id, the text naming what one writer replaces
together, the row's constructor position, the encoding of `row`, and the
fields as a sequence of tagged integers, instants and texts. Format 1 is
the only encoding; a reader refuses any other at open. A file written
before this constructor existed has no such rows and needs no migration.

Writes go through `Catalog::replace_kept` in a transaction of the write
path, one writer per group at a time. `sys::verify_findings` uses groups
`database`, `relation N` and `scrub`, N the relation's catalog id, the
scope of `spec/verify-scope/per-scope`, and one row of group `counters`
holding the next run number and finding id, so both rise across a clean
run and a reopen. Any run removes the rows of a group `relation N` when
the schema no longer has N. The engine loads kept rows at
open and holds them in memory for reads.

TODO: verifying the log from a remembered position, so a caller that verifies often does not reread the whole retained log, needs the pager to offer it.
