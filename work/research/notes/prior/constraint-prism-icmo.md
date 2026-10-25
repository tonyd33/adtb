---
title: PRISM++ integrity constraint modification operators
links:
  - {rel: prior-art, to: prior/prism-bidel}
---
Curino et al., PVLDB 4(2) 2010: ICMOs add or drop primary keys, foreign
keys and value constraints, each with a policy. Adding: CHECK (roll back
if violated) or ENFORCE (delete violators). Inverse of a drop, for old
queries: IGNORE, CHECK or ENFORCE (filter violators from answers).
https://www.vldb.org/pvldb/vol4/p117-curino.pdf
