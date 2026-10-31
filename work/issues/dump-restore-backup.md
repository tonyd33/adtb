---
created-at: 2026-10-29
status: in-progress
kind: feature
priority: 720
summary: "no dump, restore, online backup or point-in-time restore"
---

# Feature: Dump, restore, online backup and point-in-time restore

## Request

Milestone 9, interop.md 6 and 7: dump, restore, online backup, point-in-time restore. A restore keeps the file schema-owner role (interop.md 6.1). A family grant target carries its signature, so a dump round trips it.
## Proposal

## Open questions

## 2026-10-31: S4 full backup, scoped and ruled

Rulings adopted: repl/backup-heal/new-file, restore-point/one-reason, backup-proof/separate-proof, backup-target/paths, backup-damaged/named-part, backup-files/write-files, restore-epoch/bump, backup-quarantined/heap-yes-file-no, proof-host-fn/unproven, backup-occupied/refuse. interop.md 7, catalog.md 9.1 and 11.2, transactions.md 8 state them.

Design: flush and checkpoint, keep its LSN as ck. Copy pages 2 to file_pages with read_disk; a corrupt read retries under an exclusive latch, as verify's check_page does; still corrupt is listed damaged. Force the log, copy segments from batch_start(redo) to durable(). The backup's header names ck, never a later checkpoint from the file: a later checkpoint's redo would skip repair of pages copied before it. Directory layout: pages/ records of pid and page, log/ with live segment names, manifest last and forced. Manifest: kind, format, release, edition, page format range, schema version and hash, catalog version, file id, sizes, encryption list, start, checkpoint and end positions, end time, epoch history, parent hash, parts with blake3, damaged pids, quarantined objects. Dump (S5) shares the envelope, and archive (S6) the page records, log names, parent and epoch history. Nothing recycles log segments today, so backup needs no pin; a recycling track must honour running backups. Backup runs the first checkpoint under live writers.

The S4 track makes these catalog.md edits with its code, since spec_corpus and server_commands compare them with the code: in `RejectReason`, after `DumpDamaged`, `BackupDamaged { part: Text }` and `RestorePointUnavailable { oldest: Position, newest: Position }`, each with a row in the reason table and ids; the `backup` row of 9.1 needs `maintain`, and `write_files` through a server.

## 2026-10-31: next step

S4 is in round 4, change yxnwxzvu, which replaces name-based cleanup by building aside and publishing last (`Op::Rename`). Then: delta review, rebase, replace the `p.issuer` check in api/backup.rs `backup_to` with the Disk export roots that roles-adhoc landed, plus a target-inside-roots test, full check, land. S5 to S7 follow, after fault-matrix.

## 2026-10-31: tickets from round 4, after landing

`write_pages` trusts a resealed part's chunk header, which can panic or write past `file_pages`; `restore` refuses servers. `restored` and `prove_in` hash every part twice. A failed read of the test database writes a `Proved::Failed` over a true proof; `a_failed_read_of_the_test_database_records_no_verdict` holds the spec, and fault-matrix's typed cause fixes it.

From round 6, not fixed: a misnamed foreign segment, or one of another block size, reads Invalid and is ignored in recovery, so `log.rs:350` removes it; it needs hand-placed files. `list_race_panics` survives default and deep, so no test reaches the `NotFound` arm at claim's List. No test covers races or a restore interleaving on a real file.

From round 7, not fixed: `Unformatted` has no test, since a host page fault cannot be planted in a few lines. A failed log directory removal with a successful file removal leaves a log with no file, and the next create answers corrupt. The lock's postcondition needs every renamer or unlinker to hold it; restore's final rename against an opener of that name is not checked. Two-process races are tested only through a seam between open and lock.

## 2026-10-31: S4 landed

Full backup, restore and prove build aside and publish last. A log belongs to its file by `file_id` in every block. A failed open removes only what it made, and a lock is held only while the path names the locked inode. Seven rounds. The search harness is `crates/adtb-simkit/src/search.rs`. Open: the gaps listed above, an ignored red test `a_failed_read_of_the_test_database_records_no_verdict`, and two restores of one backup sharing a file id. S5 to S7 follow fault-matrix.
