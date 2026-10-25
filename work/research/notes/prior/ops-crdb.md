---
title: CockroachDB contention events, admission control and finalized upgrades
---
crdb_internal.transaction_contention_events records LOCK_WAIT and
SERIALIZATION_CONFLICT events naming blocking and waiting transaction
fingerprints and the contending key. Admission control queues CPU and
storage work by priority, then transaction start. Major upgrades roll
node by node and can be rolled back until finalized.
https://docs.cockroachlabs.com/docs/stable/monitor-and-analyze-transaction-contention
https://docs.cockroachlabs.com/docs/stable/admission-control
https://docs.cockroachlabs.com/docs/stable/upgrade-cockroach-version
