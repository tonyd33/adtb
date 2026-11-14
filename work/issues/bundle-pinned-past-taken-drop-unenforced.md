---
created-at: 2026-11-14
status: open
kind: bug
priority: 230
summary: "a bundle pinned at a version whose constraint drop already took effect registers, and its readers see rows violating that constraint"
tags:
  - engine
  - class-admission-per-path
---

# A bundle pinned before a drop that took effect reads unconstrained data

## Report

evolution.md §6.4: "Every reader sees only data that satisfies every constraint of the version it reads". Once a constraint's drop takes effect (no bundle holds it; reopen), a bundle can still be registered pinned at the old version: a probe in the review of the constraint-identity track added `no_blank` at v2, dropped it at v3 with no holder, reopened, added `no_blank` with another body at v4, then registered `web-1.0 pins 2`; `insert users { name: "" }` applied, so web-1.0's readers see a row violating a constraint of v2. Same with or without the re-add. Reproduced as an engine script; not kept.

Direction: refuse registering a bundle pinned at a version one of whose dropped constraints or checks is no longer enforced (or whose dropped objects are gone), naming them; or state in the spec that such a pin is refused. Test: the history above, expecting the registration refused.

Defect class: Authority and admission checked per entry path.
