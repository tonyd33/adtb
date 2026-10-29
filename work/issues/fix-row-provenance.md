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

## 2026-10-28: main-based executable baseline confirmed

Fresh worker rebased tests-only fixture onto verified87527a90. Exact tests-only
f6660a304bea5f4911d77d9f9dccdadefaf8f3d3 fails focused lang nextest exit100
with original Set Row holds none refusal (0.007s); no production edit preceded
this run. Retained target/provenance-baseline-corrected.log and tests-only patch
in adtb-fix-row-provenance. Initial baseline fixture mistakenly appended a second
module to RUNNING; separate original provenance-baseline.log retained. The
corrected standalone exact app schema produced the actual checker defect.

Agreed repair direction: private finite Empty/Relation/Unknown result provenance,
shared ordinary transfer rules; precheck companion solves recursive seed origins
because key x needs an origin before ordinary checking. Companion analyzes only
result flow, exhaustively handles AST and conservatively refuses unknown results.
Install stable origins into existing recursive binder checking. Final checked
body remains authority for fix Out.rows; no companion override of existing
key-preservation/snapshot/write-produced veto. Private Empty term/variable
metadata distinguishes absence from unknown without changing public Out.rows.
Full acceptance/review still pending; no source fix or passing claim yet.

### Empty-arm contextual typing prerequisite

Agent found existing synthesized union eagerly synthesizes [] without the
known sibling set type, and fix type seeding likewise treats [] as a type seed
before contextual checking. Required empty-arm neutrality cases expose this
separate checker obstacle. Retain exact failing cases before adjusting ordinary
contextual typing. Reuse existing deferred/arm_against transfer where applicable;
handle empty literals on either side and nested association, leave both-empty
without a type and incompatible nonempty operands refused. No type inference
from current data/nominal target guessing, no origin syntax exception. Final
ordinary checked-body provenance remains authority. This narrow shared typing
prerequisite is in the same checker repair track; no spec policy change.
