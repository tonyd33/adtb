---
title: TypeQL insert, put and update stages
---
Writes are pipeline stages after `match`. `put` inserts a pattern only
if it does not already match; `update` replaces single-cardinality
ownerships and role players. https://typedb.com/docs/typeql-reference/pipelines/update/
