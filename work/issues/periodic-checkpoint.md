---
created-at: 2026-11-05
status: resolved
kind: feature
priority: 420
summary: "the engine checkpoints only at open, close and backup, so dirty pages, redo and the retained log grow without bound within one session"
closed: 2026-11-07
---

# Feature: Checkpoint periodically

## Request

## Proposal

## Open questions

## 2026-11-05

Found by the readin-gate track: only flush_pages cleans the dirty page table and nothing checkpoints between open and close, so redo never advances within a session and recycling (persisted-lsn-map) bounds the log only per session. A periodic checkpointer must keep the recycle floor rule: never past the start of the current session.

The readin-gate track dropped the backup checkpoint pin: while recycling runs only at open, the session floor already keeps every segment a backup copies. A checkpoint that moves the recycle floor within a session must restore a pin at the backup's redo, and the test recycles_while_a_backup_copies_its_pages_keep_the_log_it_copies must then fail with that pin removed.

## 2026-11-06: change 1 review, a refused page pins the redo LSN

A page `Pager::damage` refuses while it is changed in memory (pager.rs, and the callers in heap/vm.rs, alloc.rs, clog.rs, heap/large.rs, verify) is evicted unwritten and keeps its dirty-table entry, so recovery replays the changes the file lacks; it never reaches `sync_data`, so every later checkpoint names the same redo and recovery's read grows with the session (reviewer's reproduction: 8 thresholds of load, 2106824 bytes read). No data lost. Change 1 narrows its bound to exclude it. Change 2 (in-session recycling) must not recycle past such a page's entry either; options: write or log the page's last good state from the log before refusing it, or carry refused pages in the checkpoint with their own replay start, kept out of the redo minimum.

## 2026-11-06: change 1 landed

`feat: checkpoint in the background once the log passes a threshold`. A checkpointer worker (storage/checkpoint.rs, with `flush_pages`, `sync_data`, `checkpoint` and `log_checkpoint` moved from pager.rs) writes every changed page and takes a fuzzy checkpoint once the log passes `Options::checkpoint_every` (default 64 MiB) since the last checkpoint record; a commit's force only wakes it. Close stops it and waits for a running cycle. The recycle floor is unmoved: the log is still recycled only at open. A page changed again after its write logs a full image at most once per checkpoint interval, and `sync_data` raises its rec_lsn to the latest image at or before its first change since the write (under atomic page writes, that change), so a hot page no longer holds the redo LSN for a session. Exception, stated in storage.md §6: a page refused while changed in memory keeps its entry and holds the redo LSN until the next open. Stated bound: a checkpoint's redo is at or past the checkpoint before it, so recovery reads at most the log since that one. Tests (checkpointer.rs, 7): a write load with crash points across it, under both write modes, asserting the bound per crash point from the file header's bytes and the run's own commits and that the hot page's rec_lsn rose; a crash search over one cycle in all three phases; a live crash tearing the hot page's write by sector after its rec_lsn was raised; a backup during a checkpoint; close during the worker's first write. Review and delta review: no data-loss blocker; the image-per-write rule (write amplification), the 2x bound and untorn crash images fixed. `check --changed` failed only in btree-crash-test-slow; full `check` on main after landing.

Filed from it: daemon-frame-reserve. Next: change 2, in-session recycling with the backup pin and the refused-page entry.

## 2026-11-07: change 2 landed, closed

`feat: recycle the log within a session`. The checkpointer recycles once the open's log readers have run (`Log::session` holds the floor until `recycle_at_open` or `publish` releases it), never under a whole quarantine, one checkpoint per cycle with the floor from the previous redo. A backup pins its redo under the log lock that appends its checkpoint until `copy_to` returns. A page refused while changed is written at refusal, except in recovery or a whole quarantine, so it holds no segment. The LSN map root's LSN is published with each root record and raised to the replayed LSN at recovery; the read-in gate's per-page entries move into the array at each recycle. Measured over 40 thresholds of log: at most 3.3 thresholds retained (bound derived as 4 intervals plus 2 segments); crash searches in both write modes reach a crash just before a recycle's first removal and between two removals, and every image reopens with all acknowledged writes, a clean verify and no leaked page. Review: no blocker; findings fixed. Note for sim-force-no-switch: in the 16-page tests an unpaced load gave the worker no turn until the load slept.

## 2026-11-07: resolved
