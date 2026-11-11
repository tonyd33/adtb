---
created-at: 2026-11-11
status: open
kind: bug
priority: 420
summary: "undoing a statement's insert panics when damage with a sound checksum makes the inserted version read as absent"
tags:
  - class-failure-flattened
---

# Undo panics on damage with a sound checksum

## Report

## 2026-11-11

`Stmt::undo`'s Kill path (write/apply.rs) reads back the version the statement inserted and calls `expect("a version the statement inserted remains present")`. Damage that leaves a page's checksum sound (a page that is a heap page but no longer holds that version, a stale image) makes the read return `Ok(None)`, and the engine panics where it met damage. `restorable` has the same shape. Damage is an operating error, never a panic (fatal-split: corruption detected is an operating error; a broken invariant is not). Found by structure-transactions step 11.

Direction: return the absence as damage (`Rot` naming the heap and tid) so it ends the transaction by its class and `fail` maps it; a plant that removes an inserted version with the checksum resealed, and a test asserting the exact reply and mark.

Defect class: Damage or failure gives a panic, hang or flattened error.
