---
created-at: 2026-10-27
status: open
kind: bug
priority: 862
summary: "two new merge refusals and the install package parse failure have no test, and the schema walk never draws a check on one constructor"
tags: [quality, lang, test]
---

# New merge refusals and the install parse failure are untested

## Report

No test reaches these messages:

- `crates/adtb-lang/src/evolve/ctor.rs:1591`: "check `{}` has no image under the merge; drop or rewrite it".
- `crates/adtb-lang/src/evolve/ctor.rs:1699`: "its constructors' field `{name}` goes to different fields".
- `crates/adtb-lang/src/evolve/mod.rs:1137-1146`: "the package `{from}` does not parse". The bad-package test at `evolve_tests.rs:387-396` fails on a check error ("nosuch"), not a parse error.

The walk's `add_check` (`crates/adtb-testkit/src/cases/walk.rs:1265`) draws only whole-relation checks, so the merge's check carrying (`merged_checks`, `carried_predicate`) is pinned only by two unit tests (`evolve_tests.rs:1397`, `:1427`).

Fix: a refusal test per message, and a walk draw of a check over one constructor.
