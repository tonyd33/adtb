---
created-at: 2026-10-28
status: in-progress
kind: bug
priority: 799
summary: key-preserving fixpoint query loses snapshot relation provenance and is refused as an update target
branch: fix-row-provenance
tags:
  - lang
  - engine
  - test
---

# Fix row provenance

## Report

Permission acceptance encountered an existing checker rejection for this snapshot
key-preserving target:

```adt
module app
type Row = { id: I64, public: Text, secret: Text }
relation left : Row key id
```

```adt
update x in (fix rows = left union rows in rows) set x
```

Actual checker refusal in the permission worker's retained initial/third logs:

```
`update x in R` ranges over rows of one relation variable, and Set Row holds none
```

Expected: check and execute as an identity update over snapshot rows of left.
Writes3.1 admits queries whose rows preserve one relation variable; writes5 applies
the same target rule to delete. Source check/expr.rs::fix binds every recursive
variable with rows=None and only copies body.rows; the recursive relation origin
never reaches the body. check/write.rs::ranged correctly refuses absent provenance.
Fresh main-based executable baseline is required to distinguish this preexisting
defect from unlanded permission changes; no main-run reproduction claimed yet.

## 2026-10-28: engine-first prerequisite

Fresh fix-row-provenance workspace onac9a52ab (code999926a8) owns shared checker
repair and regressions. Permission writer pauses production edits, preserves full
draft/failing witness, and ends for a fresh successor AFTER verified repair lands.
No permission-only exception, guessed target from nominal type/first relation,
weakened write check or unsupported-operator spec patch is authorized.

Infer snapshot relation provenance through key-preserving recursive bindings as
a general compositional finite fixed point. Preserve distinct empty/unknown/
one-relation/mixed-or-constructed cases as needed; don't assume a seed proves all
other branches safe. Support mutual recursion, aliases/filters and body binder
access. Preserve invalid mixed same-row-type relation unions, constructed rows,
write-produced rows and unanchored cases as refused. Never execute queries or
inspect current data to infer their target. No new dependency or normative rule.

Acceptance: retained main tests-only failing baseline; exact checker positives/
negatives including update and delete through fix; at least one real engine
execution through the public API checking correct target/data effects; relevant
language/write tests and simulator binaries; strict affected clippy/fmt/house/size;
one frozen full branch, fresh independent review, lead land/full main. New tests
under2s by default. Record exact source/config inputs, commands/exits/logs/timing
and incomplete scopes. Landing manual -> campaign -> this repair -> permissions;
rebase before final branch acceptance. No feature implementation overlaps its
checker writes; completed app/docs sources are disjoint. Root schedules builds.
