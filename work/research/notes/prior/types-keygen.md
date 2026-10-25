---
title: "Generated keys: sequences, UUIDv7, snowflake ids"
---
Postgres sequences: nextval never rolls back, gaps, values out of order across sessions with CACHE > 1 (https://www.postgresql.org/docs/current/sql-createsequence.html). Snowflake sequences: no gap-free guarantee, ORDER versus NOORDER (https://docs.snowflake.com/en/user-guide/querying-sequences). RFC 9562 UUIDv7: 48-bit ms timestamp, three monotonicity methods within a millisecond (https://www.rfc-editor.org/rfc/rfc9562.html). Twitter snowflake: 41-bit ms, 10-bit machine, 12-bit sequence.
