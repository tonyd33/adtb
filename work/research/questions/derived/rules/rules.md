---
title: Whether writes can cause further writes
pick: one
raised-by: [write/surface/txdata]
options:
  none:
    title: No triggers or rules; delete actions are the only engine-derived writes
    links:
      - {rel: prior-art, to: prior/derived-typedb-functions}
  change-rules:
    title: Schema rules map a change set to further changes, applied to a fixpoint
    links:
      - {rel: prior-art, to: prior/derived-pg-triggers}
      - {rel: prior-art, to: prior/derived-starburst-rules}
      - {rel: prior-art, to: prior/lang-datalog}
  single-pass:
    title: Schema rules map the submitted change set to further changes, once
  procedural:
    title: Row triggers running code with effects
    assessment: {grade: hack, why: a row loop contradicts the change-set denotation of writes}
    links:
      - {rel: prior-art, to: prior/derived-pg-triggers}
---
Queries are pure and a write is a change-set value
(`write/surface/txdata`). Delete actions (`write/delete-action/cascade`,
`retag-referrer`) already compute changes from changes. Derived data
that is a function of state belongs to views; this is for state that
must be written.

## derived/rules/none

Audit rows, counters and denormalised copies are views, or are written
by the registered transaction that makes the change. TypeDB 3 replaced
implicit rules with explicitly called functions.

## derived/rules/change-rules

```
rule audit on change d to accounts =
  from a in d.updated select insert audit_log { account: key a.new, at: tx_time(), old: a.old.balance }
```

A rule is a pure function `Changes -> Changes` over the change set and
the snapshot. Its output joins the multiple assignment and may fire
other rules; constraints check the final state once
(`write/unit/multi`). Postgres rules rewrite statements; Starburst
rules fire on transition tables.

## derived/rules/single-pass

Rule outputs do not fire rules. Termination is trivial; a rule cannot
react to another rule's audit row. CozoDB: "Triggers do not
propagate" (`kb/cozodb.md`).

## derived/rules/procedural

Postgres `BEFORE`/`AFTER ... FOR EACH ROW`; firing order is
alphabetical by name, cascades have no depth limit.
