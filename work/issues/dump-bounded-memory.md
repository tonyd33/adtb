---
created-at: 2026-11-03
status: open
kind: bug
priority: 450
summary: "dump and restore hold every entry whole in memory and restore inserts all rows as one change set, so memory grows with the database"
---

# Dump and restore in bounded memory

## Report

Found reviewing S5.1 of dump-restore-backup. crates/adtb/src/api/dump.rs and storage/dump.rs build each entry as a string, part bytes are read whole, and `fill` inserts one `Val::set` of every row in one change set. Fix direction: stream parts by region against the budget work in ops-config-resource-metrics, and insert in batches on the private database, checking constraints once at the end as the offline validation slice (S5.3) states.

## 2026-11-05: S5.5b doubles it

With S5.5b the dump holds every constructor's Arrow stream beside the text entries until `write_dump`, and per relation the row set, the stored rows, the `Batch` and the output bytes: peak memory roughly doubles over the text-only dump. Streaming each entry to its file as it is built would remove both.
