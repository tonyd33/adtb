---
created-at: 2026-10-31
status: in-progress
kind: bug
priority: 880
summary: an unscoped unique violation appends an in constructor scope to its canonical name
branch: canonical-unique-names
tags:
  - quality
  - engine
---

# Unscoped unique canonical name

## Report

Observed while running the hidden-conflict contract tests: a violation
of an unscoped `unique (slug)` was reported as
`auth::notes.unique(slug) in (Link | Plain)`. The canonical name is
`auth::notes.unique(slug)`, with no `in` suffix. The constraint and its
two report rows were otherwise correct.

`docs/spec/constraints.md` section 7.1 gives the unscoped form and also
requires an explicit scope covering every constructor to use that same
name. `crates/adtb/src/write/schema.rs::unique_name` always expands a
missing scope to all constructors and appends `in {set}`. It also fails
to canonicalize an explicit all-constructor scope to the unscoped name.
This code predates the hidden-conflict change.

Minimal reproduction shape: declare a generated-key relation with a
Text field and `unique (slug)`; insert two rows with different keys and
the same slug. Assert the exact `Violated.constraint`, report and
redaction count. Cross an omitted scope, an explicit scope naming all
constructors and a proper subset. Only the proper subset has an `in`
suffix; its constructor names are in code-point order.

Cause: the error-name formatter conflates the constructor set used for
checking with its canonical textual name. Class: copied rules. Reuse
the common canonical naming mechanism if one exists, and test the
observable name independently of the formatter under test. Do not
change the uniqueness check or weaken the exact refusal assertion.


## 2026-11-01: native comparison reproductions retained

The landed comparison controls add two runtime-data normative regressions for
`baseline::entities.unique(campaign,name)` and
`baseline::sessions.unique(campaign,number)`. Both were explicitly run and
failed on the unwanted constructor scope; exact reports, redacted count and
before/after/reopened state matched. They remain separately ignored, with the
normative bare expected names preserved. Their ignored status is an explicit
comparison limitation, not successful native constraint-name coverage.

Run both regressions with:

```sh
work/tools/suite -p adtb-bench --features native --lib -E 'test(control_tests)' --run-ignored only
```

Repair the shared naming mechanism before enabling these cases; do not normalize
names in the consumer.


## 2026-11-01: shared producer repair assigned

Workspace `canonical-unique-names`, based on `968609ec5cb0`, owns the engine
and reference evaluator's copied formatters. Move canonical unique naming to
one schema-owned provider, including the shared AST field-path helper; preserve
engine catalog reexports. Omitted and full-constructor scopes share the bare
name, proper subsets sort constructor spellings, and expression names use the
actual catalog index id. An index's position is only a local schema lookup,
not its externally printed identity. Document the shared provider in the kernel
schema boundary; no observable contract or adopted ruling changes.

Acceptance: public runtime cases distinguish omitted/full/named/proper scopes,
records, order, field paths and expression ids with independent exact expected
names/reports/state. Enable the two comparison regressions and any demonstrated
same-cause bundle regression; correct existing assertions that expect the bug.
Fresh source review overlaps meaningful production mutation and exact restore.
The author owns one final full `check` on the reviewed rebased tree, plus complete
opt-in native consumer tests/lint that default checks do not execute. The lead
compares landing inputs and reuses passes without another full run. Resource
checks precede the full check; independently ready fixture work may land first.
