---
title: Shared match failure, join points and backtracking automata
---
Maurer, Downen, Ariola, Peyton Jones, "Compiling without Continuations"
(PLDI 2017): System FJ adds `join j x = u in e` and `jump j`, typed so a
jump stands at any type and occurs only in tail contexts; transformations
push contexts into the join body instead of copying them to jumps.
OCaml's Lambda IR has the same thing as `Lstaticcatch`/`Lstaticraise`.
Maranget, "Compiling Pattern Matching to Good Decision Trees" (ML 2008):
backtracking automata are linear in code size but may retest a subterm;
decision trees never retest but may explode, and are implemented as DAGs
with maximal sharing. Detail: work/kb/maurer-join-points-2017.md,
work/kb/maranget-2008-decision-trees.md, work/kb/ocaml.md.
