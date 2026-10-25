---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 150
summary: "adtb-lang does not normalise kernel terms, and stratification, boundedness and totality are not checked over the normal form"
branch: analysis
tags: [lang]
---

# Feature: kernel normalisation and the analyses over it

## Report

`adtb-lang` owns normalisation. It does not yet perform the
normalisation of `docs/spec/semantics.md` §5.11, which removes
higher-order functions before planning. The analyses that read the
normal form are missing:

- stratification of negation and aggregation under `fix`;
- boundedness of recursive queries;
- totality and partiality.

`docs/design/kernel.md` line 25 still says it is undecided whether
`adtb-lang` or the engine normalises.

## 2026-10-25: first pass done

The first pass of normalisation and the analyses is done in workspace
`analysis`. In progress: a rebase, and a change to the `Program`
signature. The spec rulings it raised are in
`spec-fix-and-totality-rulings.md`.
