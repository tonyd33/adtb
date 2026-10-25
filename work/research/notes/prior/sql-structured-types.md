---
title: SQL:1999/2003 structured types
---
`TREAT(x AS T)`, `x IS [NOT] OF ([ONLY] T)`, `ONLY(table)`. Oracle's
`TREAT` returns NULL on a type mismatch. Postgres inheritance has `ONLY`
and `tableoid`. Gel's `[is T]` is the same narrowing as a path filter.
https://jakewheat.github.io/sql-overview/sql-2003-foundation-grammar.html
