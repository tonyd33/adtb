---
title: Debezium change data capture
---
Postgres connector: consistent snapshot, then row events from logical
decoding with before and after images, op code, txId and LSN;
optional BEGIN and END transaction events; resumes from the last LSN,
so delivery is at least once. No DDL events.
https://debezium.io/documentation/reference/stable/connectors/postgresql.html
