---
created-at: 2026-10-27
status: open
kind: bug
priority: 895
summary: "the bundle reads derivation matches Kind with wildcards, so a new kernel form is silently left out of a bundle's reader"
tags:
  - quality
  - engine
---

# The bundle reads derivation matches Kind with wildcards

## Report

`Reads::facts` (`crates/adtb/src/api/reads.rs:47-78`) ends in `_ => {}`, and `children` (`reads.rs:165-226`) treats every form it does not name as reading the row. The derived `Reader` drives readers-first and pending-drop judgement, so a new kernel form that reads a relation or writes a constructor compiles and is silently left out, letting a migration drop something a live bundle uses. `facts` also records Insert, Upsert, Replace and Rejects as writes but not `Kind::Delete`; whether that is intended is not stated next to the rule in `docs/design/schema-change.md`.

Fix: match every `Kind` without a wildcard in `facts`, and name the pass-through forms in `children`. State whether a delete counts as writing into a constructor. `plan-binder-wildcards` covers the same pattern in the planner.
