---
created-at: 2026-10-27
status: wontfix
kind: bug
priority: 896
summary: "Catalog::changes and the migration run each set up and walk a migration's steps, folding unbuilt steps, in two copies that must stay aligned"
closed: 2026-10-31
tags:
  - quality
  - engine
---

# Catalog::changes and the migration run each walk a migration's built steps

## Report

`crates/adtb/src/catalog/readers.rs:407-428` and `crates/adtb/src/catalog/mod.rs:866-899` each clone the source schema, bump the version, call `expecting`, walk `steps` and fold unbuilt steps through `steps::built`. The readers-first rules depend on both walks seeing the same steps, and nothing ties the copies together.

Fix: one iterator that yields each built step with its before and after schemas, used by both.

## 2026-10-31: closed by triage

Below the filing bar: a refactor with no observable defect.

## 2026-10-31: wontfix
