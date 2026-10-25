---
title: FoundationDB Record Layer
---
A record store is one key extent holding records of several types,
sharing one primary key space, so two records of different types cannot
share a key. A primary key may be prefixed with a record type key,
making each type's records contiguous. Indexes may span several record
types or all of them. Each store header records the meta-data version
last used; meta-data versions only increase. A new index is built
online in a write-only state, then marked readable. Transactions are
FoundationDB's: strict serializable, optimistic, at most five seconds.
https://foundationdb.github.io/fdb-record-layer/Overview.html
https://foundationdb.github.io/fdb-record-layer/SchemaEvolution.html
https://www.foundationdb.org/files/record-layer-paper.pdf
