---
title: Rules firing rules may not terminate
pick: any
raised-by: [derived/rules/change-rules]
options:
  acyclic:
    title: Refuse a rule set whose triggering graph has a cycle
    links:
      - {rel: prior-art, to: prior/derived-starburst-rules}
  monotone:
    title: Admit cycles of insert-only rules that generate no keys
    links:
      - {rel: gains, to: lang/recursion/fix}
  fuel:
    title: Abort the transaction past a fixed number of rule rounds
  live:
    title: Live with rule sets that may loop
    excludes: [derived/rules/termination/acyclic, derived/rules/termination/monotone, derived/rules/termination/fuel]
    assessment: {grade: broken, why: a commit may never finish}
---
Rule `a` writes `x`, rule `b` on `x` writes `y`, rule `c` on `y`
writes `x`.

## derived/rules/termination/acyclic

Edge from rule r to rule s when r writes a constructor relation s
fires on; schema time refuses cycles. Aiken, Widom, Hellerstein, SIGMOD
1992 analyse termination this way (detail unverified).

## derived/rules/termination/monotone

Insert-only rules over a finite set of values reach a fixpoint, as
Datalog does; `generate` would make the domain unbounded.

## derived/rules/termination/fuel

Postgres has no cascade limit; the programmer prevents recursion.
A bound turns a loop into an abort.

## derived/rules/termination/live

No check.
