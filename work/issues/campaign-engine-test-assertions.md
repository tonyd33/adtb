---
created-at: 2026-10-26
status: in-progress
kind: feature
priority: 852
summary: "campaign engine tests accept any error, check rejections by Debug text and read fields by splitting printed records"
branch: campaign-tests
tags:
  - quality
  - test
---

# Feature: campaign engine tests assert the error and the result

## Report

In `apps/campaign/tests/engine.rs`:

- `a_second_live_session_is_refused` (engine.rs:437) asserts only `refused.is_err()`, so any error passes, a bad parameter included. It should match the key violation of `unique (campaign) in Live`.
- `release_1_3_shelves_a_quest_over_release_1_1` (engine.rs:536-545) checks that the shelve and abandon updates are `Ok`, never the quest's resulting constructor.
- Rejections at engine.rs:269, :362, :456, :483 and :562 are checked as `format!("{e:?}").contains(name)`, which holds for any error whose Debug text holds the name. Match `Err(Rejected { reason: Violated { constraint, .. } })` instead.
- `field` (engine.rs:214-232) pulls values from printed record text with `find("{name}: ")` and splits on `,` and `}` without lexing strings: a text value holding `, ` or `}`, or a field whose name ends in the wanted one (`said: ` holds `id: `), returns the wrong value. Read the values from the result rows instead.

## 2026-10-28: dispatched

A fresh writer owns this in workspace campaign-tests, on main cd020ae0, beside permission-successor (adtb-lang) and the other new track. Local brief: work/briefs/brief-campaign-tests.md.
