---
created-at: 2026-10-27
status: open
kind: bug
priority: 799
summary: "a revert that enables, disables or deprecates a constructor or drops a held constraint gets past the readers-first and pending-drop rules, which match only the literal operators"
tags: [quality, engine]
---

# A revert gets past the readers-first rule and drops a constraint a live reader holds

## Report

`Readers::held` at `crates/adtb/src/catalog/readers.rs:186-201` decides what to refuse and what to keep pending by matching `op.kind` of each operator in the migration file: `Enable`, `Deprecate`, `Disable`, `DropConstraint`, `DropCheck`, and `_ => {}` for the rest. `drops_any` at `:643-655` does the same. A `revert` means its parts (the operator table of `docs/spec/evolution.md`), so a revert whose inverse enables a constructor, or drops a constraint or check, passes both rules unseen. A `drop relation` that takes its constraints with it is missed the same way.

Repro (as a sim test in `crates/adtb-sim/tests/write_states.rs`):

1. Open at the auth schema with `identities.Apple` added, then migrate `disable identities.Apple`.
2. Register reader `web-1`, pinned before Apple, reading `auth::identities` whole.
3. Migrate `module auth` / `revert <n>` where `<n>` is the disable.

Actual: the migration applies and Apple is writable while `web-1` is live.

Expected: refused as an enable before its readers, as the literal `enable identities.Apple` is.

Likewise reverting an `add constraint` that `web-1` reads drops it at once instead of keeping it pending.

`uniques_held` and `rekeys_held` already work by diffing the two schemas and are not affected.

Fix: decide enable, deprecate and disable from the write-state change between `self.source` and `next`, and constraint and check drops from catalog ids present before and absent after, as the unique rule does. Add a revert case for each rule.
