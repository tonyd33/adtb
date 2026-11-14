---
created-at: 2026-11-14
status: open
kind: bug
priority: 260
summary: "a role holding grants that runs verify is refused the witness rows of every row check's violation, whatever it may read"
tags:
  - engine
  - class-identity-by-spelling
---

# Verify refuses a check's witnesses to every role with grants

## Report

crates/adtb/src/api/command.rs (`refuse_witnesses`, about :687-705) maps a `Violation` row's catalog id with `constraint_of`, which matches only `Slot::Constraint`, and builds the readable set over `constraint_defs()` only. Checks share the constraint id space, so a row check's violation maps to nothing and is unreadable for every issuer holding grants. Reproduced on main and on the step 7 slot: with `add check named on users (scalar_len(name) > 0)`, `grant read users to dba` and a planted empty-name row, the owner's `verify` reports `Violation { constraint: constraints.key(1024) .. }` and dba's gives `Denied { "role `dba` lacks `read` on the rows of the `Violation` witnesses of `verify`" }`; with a constraint in place of the check, dba sees the rows.

Direction: one lookup that resolves both `Slot::Constraint` and `Slot::Check`, and the readable set over both kinds (a `reads_check`). Test: the probe above as a roles or commands script expecting dba to see the witness.

Defect class: Catalog rows and identity by position or spelling.
