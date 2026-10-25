# ClickHouse

kind: system
home: https://clickhouse.com/docs

## Variant and Dynamic

- Variant: UInt8 discriminator file plus one file per type; NULL is discriminator 255, cap 255 types; row-to-subcolumn offsets only in memory; granule of equal discriminators stored as 3 values. [V 2026-10-25] https://clickhouse.com/blog/a-new-powerful-json-data-type-for-clickhouse
- `v.T` subcolumn returns `Nullable(T)`; no Nullable or nested Variant inside. Union over types, not named constructors. [V 2026-10-25] https://clickhouse.com/docs/sql-reference/data-types/variant
- Dynamic default `max_types=32` (0..254); overflow into String-typed `SharedVariant` with binary-encoded type and value. [V 2026-10-25] https://clickhouse.com/docs/sql-reference/data-types/dynamic
- Dynamic cannot be indexed. [U]

## Table functions

- Table functions construct tables in FROM: file, url, s3, postgresql, mysql, numbers, iceberg, generateSeries; INSERT INTO FUNCTION file('x.csv','CSV') SELECT ... writes through them. [V 2026-10-25] https://clickhouse.com/docs/sql-reference/table-functions
