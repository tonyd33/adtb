---
title: SSI on replicas
---
Ports and Grittner (VLDB 2012, sect. 7.2): commit order under SSI is
not a serial order, so a read-only transaction on a log-shipping
replica can observe an anomaly. Plan: the primary marks safe snapshots
in the log; a replica reader takes the latest (stale), waits for the
next, or runs weaker. Munro's patch for this was withdrawn in 2019;
Postgres still refuses serializable on a standby.
https://arxiv.org/pdf/1208.4179 https://commitfest.postgresql.org/22/1799/
