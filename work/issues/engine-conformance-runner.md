---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 97
summary: the conformance suite runs only on the reference interpreter
closed: 2026-10-26
tags:
  - quality
---

# Feature: run the conformance suite on the engine

## Report

adtb-conformance runs its 265 files against the reference only. Add a second runner over the engine with a checked-in list of files the reference passes and the engine fails. The list may only shrink: a file leaving it is a pass, a file joining it fails the run. This is the first part of engine-differential-tests.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as yrnnxmxr and ulzytnsz. `crates/adtb-conformance/tests/engine-failing.txt` lists 58 of 227 files; the list reflects debug builds. Causes are filed as canon-paren-type, newtype-migration-parse (both with the front end track), planner-narrowed-call-recursion, migration-forward-refs, expression-indexes, user-aggregate-plan and conformance-round-limit.
