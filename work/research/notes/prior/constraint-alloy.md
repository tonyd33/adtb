---
title: Alloy facts
---
Alloy states invariants as `fact { all s: Session | one s.user }`,
quantified relational formulas over the whole model (Jackson, Software
Abstractions, MIT Press 2006; https://alloytools.org/). Checked by
bounded model finding, not incrementally.
