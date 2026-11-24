# Generated schemas and migrations

Schemas, statements and migrations drawn as functions of their seed alone. A drawn operator
either meets the preconditions its operator states, and the model in `migrate/` predicts the
schema and rows after it, or breaks one and must be refused naming it. The operators and their
preconditions are [evolution.md](../../../../docs/spec/evolution.md).

- The searches: `generated_migrations_keep_their_invariants` in
  `crates/adtb-testkit/tests/differential/migrations.rs`, and
  `crates/adtb-testkit/tests/differential/migration_searches.rs`.
- Under deep, `Reached::floors` fails unless every kind of `Kind::ALL` is drawn, committed and
  refused as the broken operator, every shape of `Reach::ALL` is reached, and each kind of
  `TAGGED` and `UNIMAGED` is refused for its reason; the default run needs one commit and one
  refusal. Nothing checks that `Reach::ALL` lists every variant: add a new shape to it.
- A known finding the search can draw is held off by a `SEARCH_*` switch in `mod.rs` set false,
  its doc comment stating the finding; the fix deletes the switch.
- The revert law the search runs: for a migration `Migration::exact` accepts, a revert right
  after it restores its schema and rows and a revert of that revert applies it again. Shapes for
  which `Reach::transient` holds count as exact.

Default run:
`cargo nextest run -p adtb-testkit --test differential -E 'test(=migrations::generated_migrations_keep_their_invariants)'`.
Deep:
`cargo xtask test --profile deep -E 'binary(=differential) and test(=migrations::generated_migrations_keep_their_invariants)'`.
`ADTB_DIFF_SEED` and `ADTB_DIFF_CASES` choose the seeds, as
`crates/adtb-testkit/tests/differential.rs` reads them.
