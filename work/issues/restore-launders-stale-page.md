---
created-at: 2026-11-04
status: open
kind: bug
priority: 450
summary: "A page a backup copies as damaged because its LSN is past the log end can read as sound in the restored file, whose log end passes it"
---

# Restore can launder a page past the log end

## Report

A backup refuses a page whose LSN is at or past the log's next LSN at copy time and lists it as copied damaged (crates/adtb/src/storage/backup.rs, `page_as_stored`). The backup's end is `durable()` after `flush_log`, which can pass that LSN, so in the restored file the page's LSN can fall below the restored log's end and read as sound. A checksum-damaged page stays refused after restore. The restore does return the page in its damaged list, and the restore's proof fails, so it is reported, but a later open serves the page as data.

Found in the review of the future-lsn track, 2026-11-04; not reproduced (the tests plant LSN 2^40, above any end).

## Proposal

Have a restored file refuse the manifest's damaged pages at first open, for example by quarantining their holders, so damage listed by the backup survives into the copy whatever the page's LSN.
