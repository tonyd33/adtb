# LiteFS (Fly.io)

kind: system
home: https://docs.fly.io/litefs/

## Replication

- FUSE passthrough file system intercepting SQLite writes; transactions encoded as LTX files. Replication position = TXID plus rolling checksum of the whole database. [V 2026-10-25] https://docs.fly.io/litefs/how-it-works
- Single primary elected by a Consul lease; asynchronous replication; unreplicated writes on a failed primary may be lost; an out-of-sync node after split brain is resnapshotted from the new primary. [V 2026-10-25] same
