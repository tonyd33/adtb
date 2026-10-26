---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 699
summary: "the planner recurses without bound on a comprehension over a schema function's result narrowed to a constructor"
closed: 2026-10-26
---

# Planner narrowed call recursion

## Report

## Report

`from r in shapes().Remote select r.provider` overflows the stack in release: `Planner::lift_here`, `lift_eval`, `eval`, `query`, `spine`, `chain`, `for_chain` recurse without bound. Conformance files model/01-anonymous-sums, queries/02-set-expressions, model/06-result-sums, queries/04-result-sums.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed as ryzyykky. The four conformance files stay listed for canon-paren-type only.
