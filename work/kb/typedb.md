# TypeDB

kind: system
home: https://typedb.com/docs

## Data model

- Entity types are independent; relation types declare roles played by other types; attribute types hold values. [V 2026-10-25] https://typedb.com/docs/core-concepts/typeql/entities-relations-attributes
- An attribute instance is identified by its value: two instances with one value are the same attribute. [V 2026-10-25] https://typedb.com/docs/core-concepts/typeql/entities-relations-attributes
- Entities and relations are identified through attributes they own (`owns`, `@key`). [S 2026-10-25] https://typedb.com/docs/core-concepts/typeql/entities-relations-attributes
## Constraints

- Schema constraints are annotations: `@card(n..m)`, `@key`, `@unique`, `@subkey(LABEL)`, `@values(...)`, `@range(...)`, `@regex(...)`, `@cascade`. [S 2026-10-25] https://typedb.com/docs/typeql-reference/annotations/
- TypeDB 3.0 (released 2024-12-20) replaced rules with functions. [S 2026-10-25] https://typedb.com/docs/reference/typedb-2-vs-3/diff/


## Writes

- `put` stage: inserts the whole pattern unless it already matches completely; returns existing matches otherwise. [S 2026-10-25] https://typedb.com/docs/typeql-reference/keywords/
- `update` stage replaces attribute ownerships and role players; only on owns/relates with cardinality at most 1; acts as insert where nothing matched; validated as insert stages are. [V 2026-10-25] https://typedb.com/docs/typeql-reference/pipelines/update/
- Insert errors on exceeding an upper cardinality bound, delete on going below a lower one. [V 2026-10-25] https://typedb.com/fundamentals/constraints-3-0
- TypeDB 3 checks cardinality at commit, while key/unique collisions fail at the statement. [S 2026-10-25] https://layerbase.com/blog/typedb-type-system-explained (contradicts the fundamentals page wording; primary docs not found)

## Query language (TypeQL)

- `match` reads by pattern and returns a stream of variable bindings; `fetch` projects to JSON in the shape written. [S 2026-10-25] https://typedb.com/docs/core-concepts/typeql/query-clauses/
- TypeDB 3.0 functions, e.g. `with fun mean_salary($c: company, $n: nationality) -> double`; stream-returning functions use `{}` and are called with `let` in `match`. [S 2026-10-25] https://typedb.com/fundamentals/functions-3-0/

## Type system

- Subtype instances play any role their supertype plays; unrelated types may play one role; `as` narrows roles in relation subtypes. [V 2026-10-25] https://typedb.com/applications/modeling-inheritance
- Dorn and Pribadi, type-theoretic query language, PACMMOD 2(2), 2024. [V 2026-10-25] https://dl.acm.org/doi/10.1145/3651611
- 3.0 docs: "Rules are now replaced by functions, requiring explicit functions execution instead of inference". [V 2026-10-25] https://typedb.com/docs/reference/typedb-2-vs-3/diff/
