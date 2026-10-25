---
title: Malloy
---
Semantic model of sources with declared joins (`join_one`, `join_many`)
and reusable measures; queries nest results with `nest:`. Aggregates are
computed at the grain of the source they name, so a one-to-many join
does not inflate them ("symmetric aggregates").
https://docs.malloydata.dev/documentation/language/join
