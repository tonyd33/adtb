---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 870
summary: "no history heaps, time-travel window or dropping of a removed constructor's heap"
closed: 2026-10-27
depends-on:
  - vacuum
tags:
  - engine
---

# Feature: History heaps and removed constructors

## Report

Not built: history heaps and the time-travel window
(`docs/design/mvcc.md` §10.3), and dropping a removed constructor's
heap (§10.4). Holes in a heap's directory also grow by one per removed
page until an insert fills one. Bound that too.

## 2026-10-27: done

Landed at the storage and vacuum layer, tested in
`crates/adtb-sim/tests/history.rs`:

- The record of the retention window (`mvcc::Past`): commit times per
  xid and safe snapshot markers, on one B-link tree.
  `Transactions::keep_history` turns it on; commits record their time
  and markers are kept as they are logged.
- History heaps: `vacuum::vacuum_keeping` moves versions the window
  keeps into the constructor's history heap and its key index, crash
  safe and idempotent; `vacuum::expire` removes what the window no
  longer keeps; on-access pruning and plain `vacuum` keep such
  versions in place.
- Reads of the past: `Transactions::pin` / `pin_time` (refusing with
  the oldest retained position), `history::rows` and `history::probe`.
- A removed constructor's heap: `Heap::drop_empty` frees an emptied
  heap in one pager update; its rows stay readable from history.
- Directory holes: a vacated position at the end of a level-0
  directory page that is not full leaves the directory with the holes
  before it.

Open:

- Nothing outside the tests keeps history yet: the catalog does not
  create history heaps, a history key index or the record, and no
  setting names the window (spec gap: `derived.md` §6.1 points at a
  setting `catalog.md` §10 does not list).
- `at commit` / `at instant` are refused by the planner; the checker
  types the body at the current schema, has no `pins` on an `at` block,
  and nothing raises `WrongVersion`.
- `remove constructor` still rewrites the relation and frees the old
  heap at the migration; keeping it as history needs the catalog work
  above.
- Holes inside the directory, and an emptied last level-0 page, stay
  until an insert fills them.

## 2026-10-27: storage layer landed

The storage and vacuum layer landed with the hist track: the retention record, transactions hooks, vacuum moving versions into history heaps and expiring them, past reads by `history::rows` and `probe`, `drop_empty`, and trimming directory holes at a page end, with crash tests. `docs/design/mvcc.md` sections 10.3 and 10.4 are rewritten: history key index entries are `(key, ctor, tid)`, and per-xid commit times with markers replace the commit position map. Nothing user-visible yet; the rest is in `history-catalog`, `at-block-checker`, `history-across-rewrites`, `history-commit-time-restart`, `heap-directory-interior-holes` and `spec-retention-window-setting`. Close this ticket when `history-catalog` lands.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the hcat track. What stays open is in `history-follow-ups`, `history-across-rewrites` and `heap-directory-interior-holes`.
