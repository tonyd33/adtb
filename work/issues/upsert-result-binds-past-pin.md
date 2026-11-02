---
created-at: 2026-11-02
status: resolved
kind: bug
priority: 150
summary: A pinned upsert result binding holds a row its pin cannot type
closed: 2026-11-02
---

# Upsert result binds past pin

## Report

A nonhiding pin running `let written = insert notes.Plain { .. } unless conflict on key else skip; return written` (also `written.body`, `written.id`) over a stored `Link` panics at crates/adtb/src/api/pinned.rs:41, `a carried value's constructor has a counterpart at both versions`. Seen with `fix: count a conflict branch using old as reading the relation` applied; that change does not touch the path. The branch does not use `old`, so `enable` passes.

By reading, not run: with a `Link` that has its own `body`, `written.body` reads it silently and can carry it into a later write.

Governing text: evolution.md 7.2, a live bundle is never left with a query that fails at call time because of a schema change. A kept result binding is the stored row, so it reads the relation as a use of `old` does.

## 2026-11-02: fixed

`fix: count a kept upsert result as reading the relation`. Reproduced on main: the panic, and `written.body` over `Link { url: "u", body: "theirs" }` returned `"u"`. Cause: reads counted only uses of `old`, so a bound result holding the stored row was not a read. Class: authority per entry path. Stopped by the fixture sweep over binding uses and branch forms. Full check exit 0 on the landed tree, nine mutants die, independent review found no blocker. Most live and hiding rows run only under the `deep` profile, which nothing selects by default. An unpinned misread on the same binding is on upsert-result-misread-field.

## 2026-11-02: resolved
