---
title: ClickHouse Variant type
---
Discriminator column plus one dense subcolumn per member type. Union over
types, not named constructors. UInt8 discriminator with 255 meaning
NULL, capping a Variant at 255 types. Row-to-subcolumn offsets are
rebuilt in memory, not stored. Dynamic overflows past `max_types` into
a binary-encoded shared variant.
https://clickhouse.com/docs/sql-reference/data-types/variant
