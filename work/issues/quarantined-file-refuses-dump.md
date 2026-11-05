---
created-at: 2026-11-05
status: open
kind: bug
priority: 480
summary: A file quarantined whole refuses dump in process; catalog.md 11.2 and the ruling say it opens for verify and dump
---

# A file quarantined whole cannot be dumped

## Report

## Report

catalog.md §11.2 and the ruling repl/backup-quarantined/heap-yes-file-no: a file quarantined whole opens for `verify` and `dump` only, so its data can be got out. `admit_text` (crates/adtb/src/api/mod.rs near 977) admits only a lone `verify`, so `dump` fails `Quarantined`. A user with a damaged file has no logical way out but backup, which copies the damage.

Found in dump S5.4, 2026-11-05; not reproduced by the lead.

## Proposal

Admit `dump` on a file quarantined whole, reading what is readable: decide (from the spec and the ruling) whether a dump of a quarantined file skips quarantined relations and reports them, or refuses when any dumped relation is quarantined. Test both in process and through a server.
