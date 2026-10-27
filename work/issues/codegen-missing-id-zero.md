---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 895
summary: "generated TYPE_ID, CTOR_IDS and relation ID fall back to 0 when the schema holds no catalog id"
closed: 2026-10-27
tags:
  - quality
  - lang
---

# Generated catalog ids fall back to 0 when the schema holds none

## Report

`crates/adtb-codegen/src/emit.rs:299`, `:303` and `:392` read `schema.ids.{types,ctors,relations}.get(..).copied().unwrap_or_default()` for `TYPE_ID`, `CTOR_IDS` and a relation's `ID`. A user item with no id means the replay is broken; the generator emits 0 instead of refusing, so the generated code names the wrong catalog object with no error.

Fix: return an `Error` naming the item when its id is absent.

## 2026-10-27: done

`Emitter::declared` and `Emitter::relation` return an `Error` naming the type, constructor or relation that has no catalog id. Tests: the `emit::tests` unit tests in `crates/adtb-codegen/src/emit_tests.rs`.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qwrong track.
