# Snowflake

kind: system
home: https://docs.snowflake.com/

## VARIANT

- Auto-extracts up to 200 elements per partition into columns. [S 2026-10-25] https://docs.snowflake.com/en/user-guide/semistructured-considerations

## Sequences

- No gap-free guarantee; values globally unique unless the increment's sign changes; with ORDER (not NOORDER) a statement's values exceed those of a previous statement acknowledged before it was submitted; NOORDER gives no increasing order, for concurrent insert throughput. [V 2026-10-25] https://docs.snowflake.com/en/user-guide/querying-sequences
