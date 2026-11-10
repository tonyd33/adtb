---
created-at: 2026-11-10
status: open
kind: bug
priority: 370
summary: "a migration under a relation mark reads the marked heap for its checks and leaves a mark on a dropped relation's heap, and the damage search does not judge it"
tags:
  - class-damage-unreported
---

# A migration under a relation mark

## Report

## 2026-11-10

Found by structure-damage-policy step 10 (item 5 of its brief). Nothing states what open owes a migration while one heap or index is marked: catalog.md §11.2 states calls only, embedding.md §2.1 step 8 and evolution.md §3.2 do not name quarantine, and storage.md §8 states only the whole mark. catalog.md §11.2 does state that a write is refused where its constraint or reference checks must read a quarantined constructor relation.

Probe outcomes at the step 10 change, for a migration that drops `tags`:

- `tags` heap marked and `tags` empty: the migration applies and the open returns `Ok`; the mark stays on a heap no longer in the schema.
- `tags` heap marked and holding a row: `SchemaRefused { "drop type Label", "relation tags is not empty" }`. The emptiness check read the marked heap and answered from it.
- `users` heap marked: the migration applies and reads of `users` stay refused.

Six seeds of the damage search reach this state since step 9 (see the 2026-11-10 note on migrate-under-mark-silent); its judge does not check a migration under a relation mark.

Direction: rule that a migration is refused as a call with the same reads would be, `MigrationRefused` with the `Quarantined` naming the relation as its reason, and that one reading no marked structure applies; state what becomes of a mark on a structure a migration drops; state it in embedding.md step 8 through spec-revision; teach the search's judge the rule from that text.

Defect class: Damage reported as nothing.
