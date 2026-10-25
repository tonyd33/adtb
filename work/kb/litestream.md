# Litestream

kind: system
home: https://litestream.io/

## Replication

- Holds a long-running read transaction so no other process can checkpoint and restart the SQLite WAL; copies new WAL pages and checkpoints itself. [V 2026-10-25] https://litestream.io/how-it-works/
- Packs WAL pages into LTX files with monotonically increasing TXIDs and checksums (v0.5 replaced random generation ids). Snapshot every 24 h by default; retention drops older snapshots and LTX files. [V 2026-10-25] same
- PITR: newest snapshot not past the target, then LTX files in TXID order, checking none are missing. Asynchronous replication to object storage. [V 2026-10-25] same
