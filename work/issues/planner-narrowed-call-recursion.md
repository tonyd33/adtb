---
created-at: 2026-10-26
status: open
kind: bug
priority: 699
summary: "the planner recurses without bound on a comprehension over a schema function's result narrowed to a constructor"
---

# Planner narrowed call recursion

## Report

## Report

`from r in shapes().Remote select r.provider` overflows the stack in release: `Planner::lift_here`, `lift_eval`, `eval`, `query`, `spine`, `chain`, `for_chain` recurse without bound. Conformance files model/01-anonymous-sums, queries/02-set-expressions, model/06-result-sums, queries/04-result-sums.
