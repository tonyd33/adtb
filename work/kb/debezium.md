# Debezium

kind: system
home: https://debezium.io/

## Postgres connector

- Logical decoding (pgoutput) over a replication slot; initial consistent snapshot emitting `r` events, then streaming `c`/`u`/`d` events with before/after row images and source metadata (txId, LSN); resumes from the last recorded LSN. [V 2026-10-25] https://debezium.io/documentation/reference/stable/connectors/postgresql.html
- Optional transaction metadata: BEGIN/END events with id, event count and collections; each event carries its position in its transaction. [V 2026-10-25] same
- Cannot emit DDL events, since Postgres logical decoding does not decode DDL. [V 2026-10-25] same
