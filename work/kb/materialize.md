# Materialize

kind: system
home: https://materialize.com/docs/

## Consistency

- Default isolation strict serializable; also serializable, and bounded staleness. Data fully ingested from a source is not guaranteed to appear in the next read, but once seen, later reads see it. [V 2026-10-25] https://materialize.com/docs/get-started/isolation-level/

## Views and indexes

- An index on a view keeps the full result in memory of the cluster that created it, incrementally updated; usable only in that cluster. A materialized view persists results in durable storage, readable from any cluster. [V 2026-10-25] https://materialize.com/docs/concepts/indexes/
- SUBSCRIBE emits rows with `mz_timestamp` (non-decreasing logical time) and `mz_diff` (signed multiplicity change), optional `mz_progressed` rows; `AS OF` inclusive lower bound, `UP TO` exclusive upper; AS OF needs history retained within the object's retention window. [V 2026-10-25] https://materialize.com/docs/sql/subscribe/
