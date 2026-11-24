# The write path

What a call holds of its grant is the memory section of
[execution.md](../../../../docs/design/execution.md); the checks of constraints are
[maintenance.md](../../../../docs/design/maintenance.md).

- Every collection that grows with the rows a call's items touch is a `Held` of the call's
  grant, from `Grant::held` or `Grant::hold`, until the call returns. Each row is charged once:
  one held structure owns it at a time, and it moves from the statement's result to its
  `Effects`, its `ChangeSet` and apply. Never clone a held collection into a plain one.
- A collection of rows another held collection owns borrows them, as apply's `Olds` does, or is
  built by `Grant::share`, charged only its own size, as a constraint's inputs are.
- `ChangeSet::new`, `Violations::new` and `UndoLog::new` take the grant they charge.
  `UndoLog::room` charges a heap write's undo before the write is made. `Outcome` carries counts
  per constructor, not rows.
- `Leave` decides where a call's rows are held: `Leave::admitted` in the call's grant,
  `Leave::loading` and `Leave::system` in a grant of no limit, as is a statement that scans a
  file. Uses of `Leave::admitted` and `Leave::system` are ratchet rows of
  `lint/capabilities.tql`.
- The ratchet row `write_collections` of `lint/rules.toml` counts each plain collection built in
  the write path's files; its counts only fall.
- Which refusal wins for a statement over its grant and violating a key is undecided.
- A non-test source past 1500 lines, counted after merging with main, fails the check.

Tests: the scripts under `crates/adtb-sim/scripts/execution/` of each write form past its grant;
the search `crates/adtb-sim/tests/execution/write_grant.rs`, each write form fitting n rows in a
grant and refused at 2n, `cargo nextest run -p adtb-sim --test execution -E 'test(/^write_grant::/)'`;
unit tests, `cargo nextest run -p adtb --lib -E 'test(/^write::/)'`.
