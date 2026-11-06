---
created-at: 2026-11-05
status: resolved
kind: bug
priority: 480
summary: A file quarantined whole refuses dump in process; catalog.md 11.2 and the ruling say it opens for verify and dump
closed: 2026-11-05
tags:
  - class-damage-unreported
---

# A file quarantined whole cannot be dumped

## Report

## Report

catalog.md §11.2 and the ruling repl/backup-quarantined/heap-yes-file-no: a file quarantined whole opens for `verify` and `dump` only, so its data can be got out. `admit_text` (crates/adtb/src/api/mod.rs near 977) admits only a lone `verify`, so `dump` fails `Quarantined`. A user with a damaged file has no logical way out but backup, which copies the damage.

Found in dump S5.4, 2026-11-05; not reproduced by the lead.

## Proposal

Admit `dump` on a file quarantined whole, reading what is readable: decide (from the spec and the ruling) whether a dump of a quarantined file skips quarantined relations and reports them, or refuses when any dumped relation is quarantined. Test both in process and through a server.

## 2026-11-05: fixed

`fix: dump a file quarantined whole`. Only a lone `dump` is admitted on a file quarantined whole (`admit_text`/`lone_dump`, in process and through a server). Ruling repl/dump-quarantined/prove-then-dump: such a dump first checks the catalog root, its roots heap, every engine-module relation's heap, the counters page and every commit-log page as verify does (`Engine::admit_dump`, `Catalog::read_from`); any finding refuses `Quarantined` naming the database, writing nothing; catalog.md §11.2 states it. A dump that cannot read a part fails `Quarantined` (no partial relation, §11.2 and §6.2); the heap marks of every dumped relation are checked after the entries are collected and before `write_dump`, so a dump of a partly quarantined file no longer writes a quarantined heap's rows, as it did before. A damaged deployment-facts page fails a dump as a storage error, marking nothing (the kept-rows store, storage.md §8). Tests: dump_quarantine.rs (7) and dump_server.rs; mutants killed. `check --changed` 3398 passed.

Found while testing: clog-lost-write-aborts-commit.
