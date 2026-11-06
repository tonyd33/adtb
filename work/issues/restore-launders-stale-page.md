---
created-at: 2026-11-04
status: resolved
kind: bug
priority: 450
summary: "A page a backup copies as damaged because its LSN is past the log end can read as sound in the restored file, whose log end passes it"
closed: 2026-11-05
tags:
  - class-damage-unreported
---

# Restore can launder a page past the log end

## Report

A backup refuses a page whose LSN is at or past the log's next LSN at copy time and lists it as copied damaged (crates/adtb/src/storage/backup.rs, `page_as_stored`). The backup's end is `durable()` after `flush_log`, which can pass that LSN, so in the restored file the page's LSN can fall below the restored log's end and read as sound. A checksum-damaged page stays refused after restore. The restore does return the page in its damaged list, and the restore's proof fails, so it is reported, but a later open serves the page as data.

Found in the review of the future-lsn track, 2026-11-04; not reproduced (the tests plant LSN 2^40, above any end).

## Proposal

Have a restored file refuse the manifest's damaged pages at first open, for example by quarantining their holders, so damage listed by the backup survives into the copy whatever the page's LSN.

## 2026-11-05: fixed

`fix: keep a page a backup copied as damaged damaged in the restored file`. Under the page's exclusive latch, a backup spoils its copy (`page::spoil` writes the complement of the page's own checksum) when the copied bytes fail the checksum, read unreadable, sit at or past the log end, or the page was refused `Stale` this open; the restored file refuses it as `Checksum` whatever later log growth, and quarantines its heap. A page whose copied bytes verify is copied as is even when the pager's damage map lists it (a transient `Unreadable`, a `Checksum` found after the copy). Part hashes cover the spoiled bytes; manifest format unchanged. Reproduced on main with an LSN at the recovered log end, below the backup's end. Tests in page_past_log_end.rs (the 12-entry sweep over the restored file) and backup.rs (refused pages with sound bytes; a replayed image heals a listed page); mutants killed. Review round 1 found the first mechanism (spoil every listed page at restore) spoiled sound pages; fixed by moving the spoil to the copy. Defect class: damage reported as nothing.

A backup taken before this fix has unspoiled bytes and restores as before. Left: backup-copies-lost-write (the evicted table is not consulted).
