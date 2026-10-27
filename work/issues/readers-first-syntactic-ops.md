---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 799
summary: "a revert that enables, disables or deprecates a constructor or drops a held constraint gets past the readers-first and pending-drop rules, which match only the literal operators"
closed: 2026-10-27
tags:
  - quality
  - engine
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

## 2026-10-27: done

The readers-first and pending-drop rules now judge the migration's steps, not its operator spellings. `Catalog::changes` (`crates/adtb/src/catalog/readers.rs`) walks `Source::steps` of every operator, a revert's inverse steps included, and `Changes::add` diffs consecutive schemas by catalog id: a constructor relation entering `Enabled` is an enable, `Enabled` to `Deprecated` a deprecate, `Enabled` to `Declared` a disable, a value-level constructor entering `Enabled` a value enable, and a constraint or check id that disappears a drop. The pending schema is built with the new `Source::migrate_keeping` (`crates/adtb-lang/src/evolve/keep.rs`), which puts each held object back, with its id, after the operator that removed it, so `drops_any` and the filtered migration are gone. Tests in `crates/adtb-sim/tests/write_states.rs`: `a_revert_that_enables_is_refused_while_a_reader_pinned_before_the_constructor_reads_it` (a plain disable, and a disable with a constructor rename), `a_revert_that_disables_is_refused_while_a_live_bundle_can_write_into_the_constructor`, `a_revert_that_drops_a_constraint_a_reader_holds_keeps_it_pending`, `a_revert_that_drops_a_check_a_reader_holds_keeps_it_pending`, and `a_pending_drop_is_carried_by_the_operators_after_it_in_its_own_migration`. The four revert tests failed before the fix. A constructor that comes into being already writable (a split, merge or nest product, a new type) is not an enable, as before. The `drop relation` case of the report does not arise: `drop relation` is refused while a constraint names the relation. Still open: `Catalog::emptied` in `crates/adtb/src/catalog/mod.rs` decides disable and remove from the literal operator, so a revert of an `enable` on a constructor that holds rows is not refused by it.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qread track.
