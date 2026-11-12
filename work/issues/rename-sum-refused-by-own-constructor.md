---
created-at: 2026-11-12
status: open
kind: bug
priority: 320
summary: "renaming a sum whose constructor shares the type's name is refused once that constructor is used, because the rewrite renames the uses but not the constructor"
tags:
  - class-identity-by-spelling
---

# Renaming a sum with a constructor of its name is refused

## Report

A sum `T` with a constructor named `T` (and a widened record) cannot be renamed once the constructor is used: a `match` arm `T x -> ...` becomes "`T` is not a constructor of U". Cause: the rewrite in evolve/cx.rs (around 553, the `!record` test) renames uses of the word `T` without renaming the constructor. Pre-existing on main.

Direction: rewrite type references and constructor references separately, by what each resolves to. Test: rename such a type with a match over the constructor; the constructor keeps its name and id.

Defect class: Catalog rows and identity by position or spelling.
