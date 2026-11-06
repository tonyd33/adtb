---
title: Where detected page damage becomes durable
pick: one
raised-by: [ops/halt-scope/heap]
options:
  at-detection:
    title: The detector writes the object's mark inside Pager::damage
    assessment: {grade: broken, why: "maps and marks under the caller's latches and possibly under the serialization lock; impossible in recovery before clear_tail and before the catalog loads"}
  damage-log:
    title: The pager makes each detection durable as a page-level record; object marks are derived from it
    assessment: {grade: costly, unverified: true, why: "a new durable record and a derivation every mark reader goes through; closes the class at its source"}
  drain:
    title: Detection stays in memory; one owner maps and marks every new detection at fixed latch-free points
    assessment: {grade: costly, unverified: true, why: "one marking site instead of four, but damage between detection and the next drain point lives only in memory"}
    links:
      - {rel: variant-of, to: storage/damage-mark/status-quo}
  status-quo:
    title: Callers mark as today, with a test that searches every caller of a detecting function
    assessment: {grade: hack, why: "each forgotten caller is a new instance; the search test proves only the paths it enumerates"}
---
Today a page refused by `Pager::damage` (storage/pager.rs) is held only in
`Meta::damaged`, in memory. It becomes a durable quarantine mark only where
some caller carries the error to a marking site: `mark_open_damage` on a
failed open (api/mod.rs), `quarantine_findings` after verify and scrub
(write/quarantine.rs, api/verify.rs, api/maintenance.rs), `quarantining` on
a statement's error path (api/mod.rs, api/statements.rs, write/reindex.rs).
The commit-log quarantine is a `OnceLock<PageError>` in
`Transactions` memory (mvcc/txn.rs) and becomes a mark only through
`quarantining` of an `Error::Transactions`. Every path that drops the
error between detection and a marking site absorbs the damage: a
successful open (recovery-damage-unmarked), recovery's log findings
(recovery-log-findings-dropped), background vacuum (vacuum-erases-lost-write),
the verify-fatal path.

A mark names an object (`Quarantine::Heap`, `Index`, `Database`), and the
mapping from a page to its object needs the catalog: `Database::holders`
walks every heap and tree of every relation, reading pages, repeated up to
16 times while a walk stops. The pager knows only the page id. Any option
must say who maps, when, and under which latches.

Facts every option meets:

- `Pager::damage` is called from `resident` (the read-in of every guard),
  `read_formatted`/`exclusive_formatted`, recovery's replay, verify's
  `check_page`, the allocator, heap directory and large-value store, the
  commit log and sequences: about twenty sites (`grep -rn
  'pager\.damage(\|self\.damage(' crates/adtb/src`). Its precondition is
  only that the caller holds no latch on that page; the caller may hold up
  to `LATCHES_PER_THREAD` (5) others, asserted by `check_bound`.
- `set_mark` takes the serialization lock `ser`, which a commit holds while
  it checks the marks and writes its commit record; storage.md §8 orders
  marks with commits through it. A commit writes the commit-log status page
  under `ser`, so a read-in under `ser` can detect damage.
- `change_marks` latches the marks page exclusive, logs, and waits for the
  log to be durable.
- Recovery detects damage before `clear_tail` has zeroed the discarded
  tail; storage.md §7 forbids any new log write before that. The marks
  page is loaded only after `recover` returns, and the catalog only in
  `Engine::on`.
- On a file quarantined whole, storage.md §8 lets only pages 0 and 1, the
  log and the page of marks change.
- The marks page holds `(page_size - 32) / 8` object marks, about a
  thousand at 8 KiB; `encode` asserts past it.

Prior art (work/kb/): SQL Server records every page failing an 823/824 read,
met by a query, CHECKDB or a backup, in `msdb.suspect_pages`, a page-level
table outside the damaged database, capped at 1,000 rows (sqlserver.md).
ZFS keeps a persistent per-pool error log of blocks that failed with no good
copy (zfs.md). PostgreSQL only errors the query and counts failures
(postgres.md). None of them marks an object at detection; the durable record
is page-level and object consequences are derived.

## storage/damage-mark/at-detection

`Pager::damage(pid, d)` maps `pid` to its holder and calls `quarantine`
before returning. The pager has no catalog, so the write layer registers a
callback (`Fn(Pid) -> BTreeSet<Quarantine>`) at open.

- Deadlock: the walk reads pages; one is often the page the caller holds
  latched (a B-link parent during a child's read-in), and `lock_exclusive`
  on it from the same thread never returns. The walk also adds latches past
  the bound of 5 and panics `check_bound`. `set_mark` under a commit's
  `ser` hold self-deadlocks (the lock is not reentrant).
- Open: the catalog is not loaded during `Pager::open`; recovery's
  detections would have to wait, so the option still needs a deferred path
  there and is not one place.
- Recovery: writing the marks page before `clear_tail` breaks §7.
- Quarantined whole: allowed (marks page), but the walk needs a catalog the
  open may not have read (`Engine::unread`).
- Closes, if it worked: recovery-damage-unmarked, vacuum-erases-lost-write,
  the verify-fatal hole. Does not touch recovery-log-findings-dropped (log
  damage has no page).

## storage/damage-mark/damage-log

At detection the pager appends a durable page-level record, `(pid, Damage)`,
that needs no catalog. Two placements:

1. A log record `Record::Damaged { pid, damage }` with no page, appended
   under the log mutex only. Every page change already appends under page
   latches, so this adds no lock order. It is durable at the next force; the
   caller that refuses forces it (`wait_durable`) after releasing latches,
   or lets the next commit's force carry it. A checkpoint folds the set of
   damaged pages into a pager-owned list page (beside the marks), so the
   record outlives the log segment that holds it.
2. Entries on the marks page or a chain beside it, latched as a leaf of the
   latch order: the pager then adds an order of its own, which storage.md
   §2.4 says it never does, and the latch count can exceed 5.

Placement 1 is the one with no new lock. Object marks are derived: the
mapper (`holders`) runs at a latch-free point, after the catalog opens, and
writes the object mark; until then, a reader of marks treats an unmapped
damaged page conservatively (see storage/damage-mark/derived-order).
`Pager::damaged()` reads the durable set, so verify, backup and open read
one record.

- Open: recovery's replay detections become records written after
  `clear_tail`, in the closing checkpoint's batch; recovery's log findings
  get a record of their own kind (`Damaged` keyed by LSN), which closes
  recovery-log-findings-dropped. `Engine::on` maps every unmapped record on
  every open, success or not.
- Quarantined whole: the log and the marks/list page may change, so the
  record is allowed; mapping is impossible without the catalog and nothing
  needs it, since the database mark covers every object.
- Unreadable stays out of the durable record (it is not damage, §8).
- Closes recovery-damage-unmarked, recovery-log-findings-dropped,
  vacuum-erases-lost-write (vacuum's refusal is recorded wherever the error
  goes), the verify-fatal hole, and makes the commit-log `OnceLock` a
  reader of the same record. Leaves the detection holes to
  storage/readin-gate.

## storage/damage-mark/drain

`Pager::damage` also pushes `pid` onto a pending queue. One function in the
write layer, `Database::drain_damage`, maps and marks everything pending; it
runs where no latch and no `ser` is held: at the end of every API call
(replacing `quarantining`), after every background worker step, after
`Engine::on` on every open, and after verify and scrub (replacing
`quarantine_findings` for pages).

- No new lock or latch order; the existing `set_mark` precondition holds at
  each drain point.
- Damage detected and not yet drained is in memory only: a crash between
  loses it, and the next open finds it again only if detection repeats
  (recovery chain evidence moves past at the checkpoint; a stale page is
  found again only by a gate that runs at read-in).
- Recovery's log findings still need a durable home; drain has nothing to
  mark for them.
- Closes recovery-damage-unmarked and vacuum-erases-lost-write if the
  worker loop drains; leaves recovery-log-findings-dropped.

## storage/damage-mark/status-quo

Keep the four marking sites and add a test that enumerates every function
that can return a `PageError` with damage and asserts each caller reaches a
marking site, plus fixes for each filed instance. Cheapest now; each new
caller of a reading function is a new place to forget. The search test can
only list the call graph its author enumerated, which is the shape of a
per-path check standing in for one gate.
