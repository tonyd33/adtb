---
created-at: 2026-10-29
status: open
kind: bug
priority: 240
summary: "a guarded match arm that also tests a literal or field copies its failure continuation, so normalising a write match is exponential in arms (22 arms time out)"
---

# Bug: a guarded arm copies its match failure continuation

depends on: none
needed by: none

## Report

Found by the checker-match-recursion writer (work/briefs/receipt-checker-recursion.md, local). An arm that tests a literal or a constructor field and has a guard (`| 1 if g -> e`) lowers to `If(test, If(g, e, F), F)`, with the failure continuation `F` appearing twice. Consequences:

- The normaliser recurses once per such arm and overflows a 2 MiB thread near 350 arms at opt-level 0.
- Every pass that walks the kernel term as a tree does work exponential in the number of such arms.
- In a write match whose arms insert into a relation with a generated key, normalising is exponential: 22 arms time out after 60 s.

## Fix

Bind the failure continuation once in the pattern lowering (crates/adtb-lang/src/check/pat.rs `compile`/`chain`) and refer to it from both failure positions, so the kernel term is linear in arms. The checker's kernel output changes shape, so check docs/design/kernel.md for any promise about match lowering first; if it states one, take the change through the design space. Tests: a 4000-arm guarded literal match normalises on a 2 MiB thread, and a 64-arm guarded write match with a generated key normalises in under 1 s.
