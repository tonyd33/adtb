---
title: Damage mechanism, a recommendation for where damage becomes durable and what read-in checks
links:
  - {rel: evidence, to: storage/damage-mark}
  - {rel: evidence, to: storage/readin-gate}
---
A researcher's recommendation, not a ruling. Read from code at main
(1d78ae59), nothing built or run. Prior art in work/kb/ (sqlserver.md,
oracle-database.md, postgres.md, zfs.md, btrfs.md).

## storage/readin-gate: open-map, then recycle

Recommend `storage/readin-gate/open-map` with `storage/readin-gate/recycle`.

- open-map realises the accepted `buffer/faults/stale-page/lsn-check` ruling,
  reads compared with the log, with code that exists: `read_log` already
  builds `last: Pid -> Lsn`. The gate goes in `lost_write`, the one function
  `resident`, backup (backup.rs, `sound` and the line after) and any free
  already call, so it is one gate and not one check per path. `EvictedLsns` is
  deleted.
- Its cost is one extra sequential log scan per open, linear in what has
  ever been logged: about 98 MiB for a 10 MiB file after one
  benchmark-feasibility lifecycle run. That is acceptable for now and makes
  recycle the next step. recycle is needed anyway, since nothing removes a
  segment, and it bounds the scan to the checkpoint interval plus what
  backups pin.
- Under recycle, take `backup-retention/live` for now. A page whose last
  record is recycled goes unchecked, which catalog.md §11 already says.
  `persisted-map` is the later step if detection past the retained log is
  wanted. It is Oracle's shadow tablespace, and with `map-lost-write/header-lsn`
  it is checked through the checkpoint record.
- evicted-holes leaves every page read after an open unchecked, which is the
  root of the audit's findings.

Spec and design changes: storage.md §3's lost-write row becomes every
read-in while the retained log holds the page's record; §10.3 loses the
evicted table and gains the open scan; §6 gains a recycling rule.
catalog.md §11 needs no change.

## storage/damage-mark: damage-log, as a log record folded at checkpoint

Recommend `storage/damage-mark/damage-log`, placement 1, with
`derived-order/map-under-ser` and `record-full/overflow-database`.

- The detector appends `Record::Damaged { pid, damage }` under the log mutex
  only. Every page change already appends under page latches, so this adds
  no lock or latch order, needs no catalog, and is the same at every one of
  the roughly twenty `damage` call sites.
- A checkpoint folds the set into a pager-owned list page beside the marks.
  Recovery writes its records after `clear_tail`, in the batch of its
  closing checkpoint, and records its log findings the same way.
- Object marks are derived by `holders`. It runs at open on every open
  (`mark_open_damage` moves out of the failure branch), at the end of the
  call that detected the damage, and under `ser` in a commit that sees
  unmapped records.
- at-detection is broken. Mapping under the caller's latches can wait on a
  page the same thread holds, can exceed `LATCHES_PER_THREAD`, and can take
  `ser` inside a commit. It also cannot run in recovery before `clear_tail`
  or before the catalog loads.
- drain is the cheaper fallback. It removes the four marking sites but leaves
  a crash window, and it gives recovery's log findings no durable home.

## Landing order

1. readin-gate/open-map. On its own it turns each stale read-in into an
   in-memory refusal that the next open finds again, because the map is
   rebuilt at every open. It closes damage-readin-gate,
   stale-leaf-names-freed-page (at the source),
   vacuum-erases-lost-write (vacuum's read-in is refused, so it neither
   prunes nor frees, and verify reports the page as StalePage through
   `check_page`'s known-damage branch) and backup-list-misses-late-stale.
   It probably turns clog-root-lost-write-panics into a refusal, but that is
   reasoned and needs reproducing. It weakens recovery-damage-unmarked
   (re-detected each open) but does not close it.
2. damage-mark/damage-log. It closes damage-marked-at-detection,
   recovery-damage-unmarked and recovery-log-findings-dropped, and lets the
   commit-log `OnceLock` read the same record.
3. readin-gate/recycle. It bounds step 1's open scan and the log, and
   closes no filed ticket.

Left open by all three, each a separate local fix:
verify-free-page-drops-chain (`check_page` returns early on `called_free`
before the chain findings) and unreadable-bitmap-hides-group (`called_free`
is true on `Unreadable`). After step 1 their known trigger, vacuum freeing a
stale page, no longer happens, but the early return still drops the
evidence for any other free.

## Not determined without a build

- The CPU cost of `read_log` per MiB (CRC, per-block `Vec`, record decode),
  and so the open-time cost of open-map.
- The log sizes of the sim workloads, which run in memory.
- Whether each `damage` call site holds latches or `ser` at the time. This
  was reasoned from `resident`'s callers and the commit path, not traced.
- Whether open-map turns the commit-log root panic into a refusal, and the
  reproduction of the stale directory leaf.
