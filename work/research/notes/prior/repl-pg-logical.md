---
title: Postgres logical replication and decoding
---
Row changes decoded from WAL through an output plugin into a
replication slot. DDL is not replicated; data that no longer fits the
subscriber's schema stops replication. Slot position is persisted at
checkpoint, so a crash resends changes. A slot retains WAL with no
consumer attached. Slot creation exports the snapshot the stream starts
after. https://www.postgresql.org/docs/current/logical-replication-restrictions.html
https://www.postgresql.org/docs/current/logicaldecoding-explanation.html
