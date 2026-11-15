---
created-at: 2026-11-15
status: open
kind: bug
priority: 450
summary: "in a build without debug assertions or testing, Build types a node it cannot type as Unit and builds it, and an ad hoc statement reaches the planner with no validation"
tags:
  - quality
  - lang
  - class-failure-flattened
---

# A kernel node the builder cannot type is built at Unit in a release build

## Report

Found by the quality review of reviewed..582d7af5. `kernel::Build::untypable` (crates/adtb-lang/src/kernel_build.rs:63) panics only under debug assertions or the `testing` feature; otherwise it returns `Ty::Unit`, and `typed`, `rebuilt` and `key` (lines 52, 75, 404) build the node at that type. `Build::put` (line 35) validates each node under the same cfg only. So in a release build a builder bug (a `Field` of a label the row lacks, a `Deref` of a non-key, a `key` of a non-row) yields a node typed `Unit`, a sentinel standing in for an error.

Two release paths validate afterwards and would refuse it: `Catalog::admits` (crates/adtb/src/catalog/migration.rs:196) and `registered_valid` (crates/adtb/src/api/registered/check.rs:248). An ad hoc statement has none: `check::adhoc` ends in `kernel::assert_valid` (crates/adtb-lang/src/check/mod.rs:1025), `norm::adhoc` in `assert_normal` (norm/mod.rs:270) and the planner in `assert_typed` (crates/adtb/src/plan/validate.rs:41), all no-ops in release. The ill-typed term then reaches lowering and execution, where its failure, if any, names the wrong cause.

kernel.md does not say what a consumer may assume of a term built in release.

Fix direction: make `Build` fallible where its rule can refuse (return `Result<Tm, Invalid>` from `typed` and `key`, mapped by each caller to its own internal error), or run `kernel::validate` on the ad hoc path in release as the migration and registration paths do, with an `Error::internal` naming the broken checker guarantee. Either way drop the `Ty::Unit` return. Test: an ad hoc path fed an ill-typed term in a build with the check enabled refuses as internal with its reason.
