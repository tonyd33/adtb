# Malloy

kind: language
home: https://www.malloydata.dev/

## Query language

- Joins declare cardinality: `join_one`, `join_many`, `join_cross`. [S 2026-10-25] https://docs.malloydata.dev/documentation/language/join
- `nest:` embeds a query in another, giving nested subtables, to any depth. [S 2026-10-25] https://docs.malloydata.dev/documentation/user_guides/malloy_by_example
- Symmetric aggregates: aggregates are unaffected by join fan-out. [S 2026-10-25] https://docs.malloydata.dev/documentation/user_guides/sql_experts1
