---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 192
summary: "spec examples now fail because ad hoc grants, roles and commands are checked; several examples and two rules need fixing"
closed: 2026-10-25
tags:
  - spec
---

# Bug: Spec examples and rules for checked ad hoc statements

## Report

With ad hoc statements checked, spec_examples_check went from 47 to 56 failures. Example fixes: missing `-- given` lines (security.md:118, syntax.md:1453, 1558, 1571, catalog.md:1396); operator grants written as bare examples (security.md:453, 468, syntax.md:1441 to 1449) must be marked as migration items or ad hoc text, and the syntax.md:1440 block mixes both; syntax.md:1444 grants on `invoices.Draft`, which the running Invoice lacks. Rule gaps, with my rulings: a role password parameter is `Text`; a temp relation may not reuse a schema relation's name; a temp relation cannot have a generated key (no type names it), so interop.md:421's `generate sequence` example is wrong; the type of the `dump ... encrypt` key is unstated (rule it from security.md's key material).

## 2026-10-25: resolved

In the spec with `docs: record the spec2 rulings on text, zones, dumps, temp relations and constraint names`. Point `spec2`, for Tony to adopt.

The grants still fail in migration blocks; `check-migration-operators.md` covers them.
