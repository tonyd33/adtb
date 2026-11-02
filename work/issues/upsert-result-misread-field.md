---
created-at: 2026-11-02
status: resolved
kind: bug
priority: 120
summary: "A filtered conflict branch or ranged update leaves a kept row typed as the written constructor, reading the wrong field"
closed: 2026-11-02
---

# Upsert result misread field

## Report

From the review of `fix: count a kept upsert result as reading the relation`, by reading, not run. No migration or pin needed.

Stored `Link { id: notes.key(1), url: "u" }` under `type Note { id: Key notes } = Plain { body: Text } | Link { url: Text }`:

`transact let w = insert notes.Plain { id: notes.key(1), body: "new" } unless conflict on key else update old where 1 == 2 set new; return w.body`

is expected by the reading to return `"u"`. Same for `update x in notes.Plain where x.body == "zzz" set Plain { body: "x" }`. crates/adtb-lang/src/check/write.rs `Effect::wrote` claims a write when zero rows match, so `upsert_binding` does not join `old_ty`, crates/adtb/src/write/eval.rs binds the stored row, and crates/adtb/src/exec/types.rs reads position 1. Direction: a filtered or ranged update adds an empty `Done` outcome.

crates/adtb/src/api/reads.rs `replaces` and the checker's `Effect` are two analyses of whether a branch can keep the row; they disagree here, and `replaces` is right.

## 2026-11-02: fixed

`fix: type a conflict branch that can match nothing as keeping the row`. Reproduced on main with adtb-probe: reference refused with `Internal`, engine copied `url` into `body`. Cause: `upsert_binding` judged a kept row from `Effect`, where any update counted as a write. Class: wildcards and copied rules. Stopped by one answer, `Terms::replaces`, shared by the checker and `keeping_upsert`, and a sweep of 35 branch forms over two stored constructors on reference and engine. Both mutants die. Full check exit 0 on the landed tree; independent review found no blocker.

Behaviour: reading a constructor field off a filtered or ranged branch's result on a sum is refused even when the row would match, as the rule requires.

## 2026-11-02: resolved
