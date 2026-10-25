---
title: Viewstamped Replication and TigerBeetle
---
Liskov and Cowling 2012 (unverified, PDF unreachable): primary-backup
with view changes, primary chosen from the view number. TigerBeetle
runs VSR with replication, view-change and nack quorums; a primary
assigns op and timestamp; protocol-aware recovery repairs a corrupt
local prepare or block from peers.
https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/internals/vsr.md
