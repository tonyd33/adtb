---
created-at: 2026-11-05
status: open
kind: bug
priority: 700
summary: work/tools/mutants counts a mutant site that rustfmt splits across lines as several sites and refuses it
---

# mutants miscounts a site rustfmt splits

## Report

## Report

`work/tools/mutants` counts sites with a line-oriented `rg -Uo ... | wc -l`, so a `mutant!(ID, original, mutated)` call that rustfmt wraps over several lines counts as several sites and the tool refuses the build. Authors work around it by keeping each site on one line. Seen in the read-blame track, 2026-11-05.

## Proposal

Count sites by the macro's ID token (one match per `mutant!(` opening with its ID), not by matched lines; add a case with a wrapped site to the tool's self-test.
