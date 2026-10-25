# Feldera

kind: system
home: https://docs.feldera.com

## Views

- Implements DBSP. By default stores only data needed to compute future outputs; a MATERIALIZED table or view keeps a complete snapshot, increasing storage, and is not a performance optimization. [V 2026-10-25] https://docs.feldera.com/sql/materialized
- LATENESS annotations let it garbage-collect old records for bounded storage over unbounded streams; materializing such a table forces it to keep the whole history. [V 2026-10-25] https://docs.feldera.com/sql/materialized
