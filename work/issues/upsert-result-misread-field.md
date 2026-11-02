---
created-at: 2026-11-02
status: open
kind: bug
priority: 120
summary: "A filtered conflict branch or ranged update leaves a kept row typed as the written constructor, reading the wrong field"
---

# Upsert result misread field

## Report

From the review of `fix: count a kept upsert result as reading the relation`, by reading, not run. No migration or pin needed.

Stored `Link { id: notes.key(1), url: "u" }` under `type Note { id: Key notes } = Plain { body: Text } | Link { url: Text }`:

`transact let w = insert notes.Plain { id: notes.key(1), body: "new" } unless conflict on key else update old where 1 == 2 set new; return w.body`

is expected by the reading to return `"u"`. Same for `update x in notes.Plain where x.body == "zzz" set Plain { body: "x" }`. crates/adtb-lang/src/check/write.rs `Effect::wrote` claims a write when zero rows match, so `upsert_binding` does not join `old_ty`, crates/adtb/src/write/eval.rs binds the stored row, and crates/adtb/src/exec/types.rs reads position 1. Direction: a filtered or ranged update adds an empty `Done` outcome.

crates/adtb/src/api/reads.rs `replaces` and the checker's `Effect` are two analyses of whether a branch can keep the row; they disagree here, and `replaces` is right.
