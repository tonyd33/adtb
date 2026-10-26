# Structural verification

`adtb::verify` checks a whole database file for structural damage: the
pages and the log as the pager checks them, every heap, large-value
store, B-link tree and commit log, the routing from index entries to
heap slots, and the allocation bitmap against what refers to each page.
It provides the structural checks of `verify`
([../spec/catalog.md §11.1](../spec/catalog.md#111-verification)) to
the catalog, to the maintenance commands and to the test harnesses.
The cross and logical checks of that section are not here.

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
- No write runs while `verify` or `repair` runs.

When the catalog exists it builds the inventory from its own records;
the catalog's pages are then one more structure.

## 2. Findings

`verify(pager, inventory)` returns a report:

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

`repair(pager, report)` frees every page the report found `Leaked`,
and returns how many.

Preconditions: the report is complete and was made with no write
since. `repair` panics on an incomplete report.

Postcondition: a `verify` right after reports no `Leaked` page.

A crash between allocating a page and linking it, or between unlinking
a page and freeing it, leaves such a leak
([storage.md §2.3](storage.md#23-page-access)). After every crash and
recovery, `verify` finds nothing but leaked pages, and after `repair`
nothing at all.
