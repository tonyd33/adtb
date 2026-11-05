---
created-at: 2026-11-05
status: open
kind: bug
priority: 430
summary: "a backup copies a page whose write was lost (evicted LSN newer than the file's) with a valid checksum, unlisted or listed but unspoiled, and the restore serves the old version"
---

# Backup copies a lost write as sound

## Report

## Report

`trusted` (crates/adtb/src/storage/backup.rs, in `page_as_stored`) treats a copy as untrusted only when the page is already in the pager's damaged map as `Stale`. It never compares the copy's LSN with the pool's evicted table (`meta.evicted`), which `Pager::resident` uses to refuse a lost write. Since the change's records precede the backup's redo point (`flush_pages` dropped the page from the dirty table), the restore serves the old version silently.

Reproduced in the scoped review of restore-stale, 2026-11-05, with a temporary test built on `a_lost_write_is_caught_when_the_page_is_read_back` (`lost_write_p: 1.0` on the victim's flush, reads that evict it, `Pager::backup`, `restore_file`, read from the copy):
- A: no reader touched the page before the backup. The source refuses it `Stale { expected: 237624, found: 124472 }`; the manifest lists `[]`; the restored read gives `Ok(0)` where the acknowledged byte was 7.
- B: a reader refuses it inside `visit` after the copy. The manifest lists `[Pid(4)]` (through `damaged.extend(self.damaged().keys())`), and the restored read still gives `Ok(0)`.

Same gap on main before the restore-stale fix; not a regression of it.

## Proposal

In `trusted`, also treat a copy as stale when `meta.evicted.get(pid)` is greater than `page::lsn(buf)`, as `resident` does (the reviewer's temporary edit made A and B restore as `Err(Checksum)`). Also: a clean resident page whose pool LSN is newer than the file's copy (compare under the latch); a `Stale` that `verify` finds from a broken log chain after the copy, where the manifest lists a page copied unspoiled: either list only pages spoiled plus those refused before the copy, or state that the list may include pages copied sound. Tests: A and B as above, each failing without the check.
