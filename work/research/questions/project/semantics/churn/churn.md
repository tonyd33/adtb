---
title: Mechanising a language that is still being designed
pick: any
raised-by: [project/semantics/mechanised]
options:
  core-first:
    title: Mechanise only the kernel calculus the surface desugars to
    needs: [project/semantics/kernel/core]
  after-freeze:
    title: Mechanise once the first edition is frozen
  live:
    title: Keep proofs in step with every design change
    excludes: [project/semantics/churn/core-first, project/semantics/churn/after-freeze]
---
Every ruling that changes the language rewrites definitions and breaks
proofs. Cedar's proof-to-model ratio is about 3.4:1 (arXiv:2407.01688).

## project/semantics/churn/core-first

Surface features (paths, backlinks, or-patterns, upsert) are sugar
over a small calculus; the proofs live on the calculus and survive
surface changes.

## project/semantics/churn/after-freeze

Paper semantics during design, mechanised proofs for an edition. The
gap between them is a window for unproven rewrites in the planner.

## project/semantics/churn/live

Each design change pays its proof cost immediately.
