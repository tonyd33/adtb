---
title: How metrics leave the engine
pick: some
options:
  relations:
    title: Statistics are system relations queried with the language
    needs: [engine/catalog-relations]
    links:
      - {rel: prior-art, to: prior/ops-pg-stats}
      - {rel: prior-art, to: prior/ops-mysql-pfs}
  otel:
    title: The server exports OpenTelemetry metrics and traces
    links:
      - {rel: prior-art, to: prior/ops-otel-db}
  scrape:
    title: The server serves a Prometheus text endpoint
  callback:
    title: The embedded library hands counters to the host through an API
    needs: [surface/wire/embedded]
---
The granularity is the constructor heap: rows, pages, dead versions,
index entries per constructor of a sum, since each constructor is a
heap (settled). A per-relation number is the sum over its heaps.

```
from h in sys::heaps where h.sum == types.Identity
select { ctor: h.ctor, live_rows: h.live_rows, dead_versions: h.dead_versions,
         pages: h.pages, last_vacuum: h.last_vacuum }
```

## ops/expose/relations

pg_stat_* views, performance_schema tables, SQLite `pragma_*` table
functions. One query language for data and operations.

## ops/expose/otel

Push to a collector. Standard names for host metrics; database
metrics are engine-specific.

## ops/expose/scrape

Pull. A text format any monitoring stack reads without a driver.

## ops/expose/callback

For embedding, where there is no port: the host reads counters and
forwards them to its own telemetry.

```rust
let m = db.metrics();            // snapshot of counters
otel_meter.record(m.heap("identities.Google").dead_versions);
```
