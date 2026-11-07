---
created-at: 2026-11-06
status: open
kind: bug
priority: 350
summary: a rekey that changes another relation's key through its reference field may leave that relation's keys held elsewhere stale
tags:
  - class-none
---

# Bug: a rekey leaves dependent held keys stale

## Report

Seen by the migration-identity-sweep change 2 author (2026-11-06), not reproduced. A rekey of `docs` changes the key of `doc_tags`, a relation keyed by `doc: Key docs`. A `Key doc_tags` held in a third relation's field then names the old key. The carry check does not require that field to be listed, and nobody checked whether anything refuses the migration or rewrites the held key.

Expected: either the rekey rewrites every `Key doc_tags` held anywhere (transitively, as it rewrites `Key docs` it carries), or it refuses naming the field it would leave stale. A stale key stored silently is a broken reference the constraints no longer see.

First step: a script that rekeys `docs` with a third relation holding `Key doc_tags`, asserting the exact rows or the exact refusal.
