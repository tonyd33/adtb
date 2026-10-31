---
created-at: 2026-10-29
status: in-progress
branch: backup
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
