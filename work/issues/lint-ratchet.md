---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 90
summary: "the workspace has no lint configuration beyond clippy's default set"
branch: adtb-lint
tags:
  - quality
---

# Feature: workspace lint ratchet

## Report

Add `[workspace.lints]` to the root Cargo.toml and `lints.workspace = true` to every crate. Deny: unsafe_op_in_unsafe_fn, unused_qualifications, dbg_macro, todo, unimplemented, print_stdout, print_stderr, undocumented_unsafe_blocks, multiple_unsafe_ops_per_block, allow_attributes, allow_attributes_without_reason, self_named_module_files, let_underscore_must_use, unwrap_used, redundant_clone, semicolon_if_nothing_returned, uninlined_format_args, manual_let_else, unnested_or_patterns, ignored_unit_patterns, explicit_iter_loop, redundant_closure_for_method_calls, needless_raw_string_hashes, cast_lossless, match_same_arms, stable_sort_primitive, range_plus_one, too_many_lines, cognitive_complexity, too_many_arguments. In clippy.toml: allow-unwrap-in-tests, allow-print-in-tests, too-many-lines-threshold = 150, cognitive-complexity-threshold = 25. Fix the mechanical sites (most take `clippy --fix`); put today's oversized and over-complex functions and the two eight-argument functions in plan/lower.rs under `#[expect(lint, reason = "...")]`. Convert every `#[allow]` to `#[expect]` with a reason. Handle the discarded abort error in write/mod.rs rather than expecting it away.
