---
created-at: 2026-11-11
status: open
kind: bug
priority: 460
summary: "a rejected statement is spelled both write::Error::Rejected and write::Error::Exec(exec::Error::Rejected), so a match naming one misses the other"
tags:
  - class-identity-by-spelling
---

# One rejection, two spellings across write and exec

## Report

## 2026-11-11

After structure-transactions step 10, `write::Error` holds exec failures as `Exec(exec::Error)` and keeps one spelling for each shared failure, except `Rejected`: write's `Rejected(Rejection)` carries violations, keys, source and line, and exec's `Rejected(String)` carries a `reject` text. The two overlap at `BadInput { source: "", line: 0 }`, and the three source-reject sites in write/eval.rs match both spellings by hand. The compiler does not flag a new match that names only one, so it silently misses the other. `Corrupt(write::Rot)` and `Exec(Corrupt(exec::Rot))` are a second pair meaning decoded damage; their consumers today are `class()`, `detail` and `of_mapped`.

Direction: give the rejection one type both layers build (exec's `reject` text as one `Rejection` case, or a `Rejection` owned below write that exec can name), so `Rejected` has a single spelling; do the same for decoded damage or state why two remain.

Defect class: Identity by spelling: a string or position stands in for the identity the type system could hold.
