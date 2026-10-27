---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 850
summary: no tests check the schema operator laws or walk generated schemas
closed: 2026-10-27
depends-on:
  - operator-log
  - engine-differential-tests
tags:
  - engine
  - test
---

# Feature: schema operator law tests and schema walks

## Report

Implement `docs/design/oracle.md` §5.7 and §5.8 over the generator of
§4: operator laws (inverse, data map, declared loss) over generated
databases, and the schema walk generator. Milestone 8 exits when the
laws hold over generated databases.

Prerequisites: `operator-log.md`, `engine-differential-tests.md`.

## 2026-10-26: done

The schema walk generator (`crates/adtb-testkit/src/cases/walk.rs`) draws 32 operator kinds: add relation, add constructor, enable, deprecate, disable, remove constructor, the four renames but sets, splits by a discriminator, by a `Bool` field, into one product and lossy, merge, nest, unnest, add, drop, rename and move fields up and down, alter field, add and drop index, unique, default and check, and redefine fn. Each step states its operators and its inverse: the revert with its fills and the database it restores, or that it is refused. `crates/adtb-testkit/tests/differential.rs` checks each step's data map against the reference (`schema_walks_keep_engine_equal_to_reference`) and its revert and the revert of that revert (`schema_operators_keep_their_laws`); `walks_draw_every_operator` checks the generator reaches every kind.

Open:
- Remove constructor, drop unique and drop check stage as pending, so the default sweeps skip them; the ignored sweeps `every_*` draw them and pass at the seeds tried.
- Add and drop default have no resolved form (`resolved-operators`); skipped, and `an_added_default_fills_a_later_insert_and_its_revert_drops_it` in `crates/adtb-sim/tests/oplog.rs` is ignored.
- The revert of a split by a `Bool` field or into one product is refused: the derived inverse merge has no discriminator. Ignored tests `reverting_a_split_by_a_bool_field_merges_the_rows_back_with_the_field`, `reverting_a_split_into_one_product_merges_the_rows_back`.
- The revert of a move down to several constructors asks a fill for every listed constructor but the first and drops their values. Ignored test `reverting_a_move_down_to_two_constructors_keeps_their_values_and_fills_the_others`.
- Translation of queries pinned to a version (`q(D) == q'(op(D))`) waits on bundles.
- Rekey, sets, constraints by query, drop relation and drop type are not drawn.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the laws track. The laws hold for every settled operator; broken ones are pinned in `spec-split-merge-invertibility-names`, `move-down-several-revert` and `resolved-operators`; other gaps are in `operator-law-follow-ups`, and the pinned query laws in `version-translation`.

## 2026-10-27: resolved forms

Every operator now has a resolved form (resops track). `store` writes catalog rows and the view is computed on read; `script`, `install` and `adopt tzdb` are still refused, see `script-in-engine` and `extensions-and-tzdb-rows`.
