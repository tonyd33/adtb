# The adtb manual

adtb is a database engine whose relations are sums: a relation holds
rows of one type, and each row is one of that type's constructors. This
manual teaches you to use it. It describes what the engine runs today;
the normative rules are in [the specification](../spec/README.md), and
where the two differ the specification is right.

Every example on these pages is run by the test suite against the
`adtb` shell, and the output shown is the output it prints.

## Tutorial

Read these in order. Each page starts from a database you can create
in two commands, so you can also start at any of them.

- [Getting started](getting-started.md): a quick tour, from nothing to
  a migrated database.
- [1. Declare a schema](tutorial/1-schema.md): modules, record types,
  relations, keys and base types.
- [2. Sums](tutorial/2-sums.md): constructors, header fields,
  enumerations and value-level sums.
- [3. Queries](tutorial/3-queries.md): comprehensions, joins, paths,
  aggregates, grouping and ordering.
- [4. Match](tutorial/4-match.md): patterns, exhaustiveness, guards and
  named constructor sets.
- [5. References and constraints](tutorial/5-references-and-constraints.md):
  `Key` fields, delete actions, uniques, checks and constraints.
- [6. Transactions](tutorial/6-transactions.md): statements, `transact`
  blocks and interactive transactions.
- [7. Migrations](tutorial/7-migrations.md): changing a database that
  holds data.

## Guides

- [Modelling with sums](guides/modelling-with-sums.md)
- [Evolving a schema](guides/evolving-a-schema.md)
- [Writing constraints](guides/writing-constraints.md)
- [Files and CSV scans](guides/files-and-csv.md)

## Concepts

- [Relations are sums](concepts/relations-as-sums.md)
- [No NULL](concepts/no-null.md)
- [Keys and references](concepts/keys-and-references.md)
- [Serializable transactions](concepts/serializable-transactions.md)
- [The migration log](concepts/migration-log.md)

## Reference

- [The shell](reference/shell.md)
- [The language](reference/language.md)
- [The prelude](reference/prelude.md)

## Not yet in this manual

These pages are planned. Some wait for the engine to run what the
specification describes, the rest for a test that runs their examples:

- Calling adtb from Rust: opening a database in process, calls,
  transactions and errors through the embedding API. The engine runs
  it; the page waits for a test that compiles its examples.
- Bundles: registering a bundle of queries and transactions, pinning
  it to a schema version, and retiring it. The shell has no bundles,
  and `register bundle` is refused.
- Errors: every error a user can see, what it means and what to do.
- Operations: `print schema`, `verify`, `export`, dump and restore,
  backups and `reindex`, which the engine refuses today with "does not
  run in this engine yet".
- Migration scripts, defaults, computed fields and stored views: the
  `script`, `add default`, `add computed` and `store` operators, which
  the engine refuses with "this operator has no resolved form yet".
