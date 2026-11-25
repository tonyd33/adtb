# Migrations in the checker

The contract with the catalog and the planner is
[schema-change.md](../../../../docs/design/schema-change.md); what users observe of a migration,
its refusals and its revert is [evolution.md](../../../../docs/spec/evolution.md).

- `model/` holds a source as declarations keyed by catalog id and applies each operator as a
  function of it.
- Text an operator keeps, a body, a default or a check, is a `Term`: each name in it is held as
  the ids it resolves to and printed under the name the model gives those ids now. Never keep a
  written name as a `String`.
- An operator resolves a type, relation or named set through `Resolving`, which ties the name's
  span to its id. Where the lookup misses it calls `Model::unheld` before refusing, so a name a
  held declaration takes refuses with the declarations' first failure.
- A revert builds its inverses in `model/inverse/`. `Transient::of` decides from the states,
  before any inverse is built, which steps touch only objects the migration both adds and drops;
  those steps are cancelled and their inverses never built, since an inverse can refuse while
  built, as one demanding a fill does.
- The revert law: a revert right after an exact migration restores its schema and rows, and a
  revert of that revert applies it again.
- An inverse holds after any later migration too: it restores what its migration dropped by id
  from the model before it, never by a name re-read now, and refuses a later change it cannot
  undo with a stated reason. Test a revert of an earlier migration after later ones.
- A finding the migration search can draw but no fix has met is held off by a `SEARCH_*` switch
  in `crates/adtb-testkit/src/schemas/mod.rs`; the fix deletes its switch.

Tests: `cargo nextest run -p adtb-lang -E 'test(/^evolve::/)'`; the engine scripts of the log
and of bundles, `cargo conformance --file-name crates/adtb-sim/scripts/oplog` and
`crates/adtb-sim/scripts/bundles/`; the generated migration search, its floors and switches, in
`crates/adtb-testkit/src/schemas/AGENTS.md`.
