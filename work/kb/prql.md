# PRQL

kind: language
home: https://prql-lang.org/

## Query language

- Transforms take a relation and produce a relation, chained into a pipeline. [S 2026-10-25] https://prql-lang.org/book/reference/stdlib/transforms/
- `group {keys} (pipeline)` applies a pipeline to each partition; most often with `aggregate {...}`, which keeps only the aggregate tuple's columns. [S 2026-10-25] https://prql-lang.org/book/reference/stdlib/transforms/group.html

## Types

- Sum types in spec (`type status = (paid = () || unpaid = float || ...)`); type system under development; no match. [V 2026-10-25] https://prql-lang.org/book/reference/spec/type-system.html
