---
extends: current
proposed: true
accept:
  lang/patterns/failure-sharing/guard-vars/second-scope: {title: Check the guard under its own binder variables, reason: Keeps one binding site per variable as kernel.md section 2 states; no term copying.}
  lang/patterns/failure-sharing/retest: {title: 'Lower each arm to one condition and one body, retesting the pattern in the body', reason: 'Linear kernel terms for every pattern shape with no new kernel form and no change to any consumer: F appears once per arm, so the normaliser memo refusing draws and every tree walker stop mattering for matches. It is the backtracking-automaton trade (linear size, retested subterms), and the current lowering already retests the scrutinee per arm. memo cannot fix path-sensitive passes (delta guards, planner, code generator) without rebuilding join points inside each; join is the principled general form but costs every consumer plus disjunctive path conditions in the planner and incremental checks, for a problem the lowering can avoid creating.'}
  lang/patterns/failure-sharing/retest-cost/live: {title: Live with one repeated total test per nested constructor, reason: One tag read per nested constructor in the arm that matches; failing arms cost what they cost today.}
  lang/patterns/failure-sharing/unreachable-meaning/live: {title: Restate Unreachable as a position the checker proves unreached, reason: One sentence in the design file; reaching it stays a broken invariant.}
---
