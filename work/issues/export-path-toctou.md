---
created-at: 2026-11-05
status: open
kind: bug
priority: 460
summary: "backup and dump through a server check the export roots once by path, then write by path; a directory swapped for a symlink in between escapes the roots"
---

# Export root check is time of check, not time of use

## Report

## Report

`Engine::confine` (crates/adtb/src/api/backup.rs) resolves the target once against the export roots (`Disk::exports_beside` in crates/adtb-file/src/lib.rs, canonicalizing each prefix). `storage::write_dump` and backup's writer then create the parent, the aside directory and publish by path. A local actor with write access to the export tree who swaps a directory for a symlink between the check and the writes makes the server write outside its export roots. Also: in the simulator the check sees the host file system and the writes the sim's, so no sim test can exercise the escape.

Found in the review of dump S5.4, 2026-11-05; reasoned, not reproduced.

## Proposal

Open the export root as a directory handle and do every create, write and rename relative to it with no-follow semantics (openat, O_NOFOLLOW, renameat), shared by backup and dump; route the check through the runtime's file system so the sim sees what production does.
