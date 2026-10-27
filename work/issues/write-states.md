---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 810
summary: "no write states: readers-first, pending drops and staged jobs do not exist"
closed: 2026-10-27
depends-on:
  - operator-log
tags:
  - engine
---

# Feature: write states, readers first and staged jobs

## Report

Implement `docs/spec/evolution.md` §6 and §8 and
`docs/design/schema-change.md` §2: write states, readers-first, pending
drops, staged jobs.

Prerequisite: `operator-log.md`.

## 2026-10-26: done

Landed: `catalog::Reader`, `Catalog::register`, `Catalog::release` and `Catalog::pending_drops` as the in-process statement of live readers. `Catalog::migrate` refuses an `enable` a reader blocks (relation and value-level, `hide new constructors` honoured), a `deprecate` or `disable` of a constructor relation a reader writes into, and a drop of a unique or a relaxing rekey a reader's version holds. A held `drop constraint` or `drop check` applies and stays pending: validation and admission run against a source that keeps it, later operators carry it, a migration leaving it without an image or failing its recheck is refused naming it, and it takes effect on the release of its last holder. `deprecate` and `disable` are refused while a delete action retags into the constructor; `disable` and `remove` of a value-level constructor are refused while a field stores it. A failed unique names the shared keys. Tests in `crates/adtb-sim/tests/write_states.rs`.

Open: a dropped unique and the old key of a relaxing rekey are refused while a reader holds them instead of held pending (`a_dropped_unique_stays_pending_while_a_reader_pinned_before_the_drop_is_live` fails). The write path an open binds uses the version's program; nothing is pending at an embedded open, but a server keeping drops pending must bind the enforced program. An `arm` clause naming a pending-dropped constraint is refused by the version's source. Staged jobs run inside the migration's one transaction (stated in `docs/design/schema-change.md` §2); `sys::jobs`, `sys::pending_drops` rows, `MigrationInProgress` and `BlockingMigration` notices need a server. Deriving readers from registered bundles and §8.4 supersession belong to `bundles`.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the wstates track. Open items moved to `pending-unique-drops` and `write-states-follow-ups`.
