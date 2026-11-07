---
created-at: 2026-11-06
status: open
kind: feature
priority: 200
summary: move 29 adtb-sim tests that assert refusal detail onto engine scripts
---

# Feature: refusal-detail tests onto engine scripts

## Request

The conformance move of 2026-11-06 (test-weight-and-script-runner) found 29 adtb-sim tests that fit no conformance file at equal strength: each asserts a refusal's payload (admission detail, BadInput source, line and detail, KeyViolation relation and key, a UserError message) or needs a second migration, and conformance `error C` compares the reason constructor only. They are engine-only and fit crates/adtb-sim/scripts with `error C VALUE` and `migrate`.

constraints.rs (17):
1. a_join_on_a_field_one_constructor_has_no_index_on_is_refused_naming_the_set
2. a_constraint_joining_on_a_field_no_index_leads_with_is_refused_naming_it
3. an_inclusion_without_an_index_on_its_source_is_refused_naming_the_index
4. only_the_relations_whose_changes_are_unbounded_are_named
5. a_read_no_field_connects_to_the_change_is_refused
6. recursion_is_outside_the_bound
7. min_and_max_need_an_index_on_the_field_within_the_group
8. an_exclusion_over_a_range_needs_an_interval_index_after_its_equality_fields
9. a_migration_declaring_a_constraint_outside_the_bound_is_refused
10. dropping_an_index_an_admitted_constraint_needs_is_refused_naming_the_constraint
11. dropping_the_declared_index_leading_a_reference_field_brings_back_the_implied_one
12. a_transition_read_under_negation_is_refused_naming_it
13. a_transition_read_inside_an_aggregate_is_refused_naming_it
14. a_deleted_read_of_a_constructor_under_not_is_refused_naming_its_constructor
15. a_transition_read_in_an_if_condition_is_refused
16. a_migration_declaring_a_transition_read_under_negation_is_refused_naming_the_read
17. storing_a_view_with_no_index_bounded_maintenance_is_refused_naming_the_index (ignored)

files.rs (8):
18. a_missing_column_is_bad_input_naming_the_file_row_and_column
19. a_cell_that_does_not_convert_is_bad_input_naming_its_row_and_column
20. a_glob_numbers_records_across_its_files_in_name_order
21. a_file_that_cannot_be_read_is_bad_input_naming_it_at_line_zero
22. a_load_without_rejects_fails_with_bad_input_on_a_failed_record
23. a_reject_in_a_load_without_rejects_fails_with_bad_input_naming_its_line
24. a_line_that_is_not_json_does_not_convert
25. a_temp_relation_key_is_checked

api.rs (4):
26. a_failing_item_applies_nothing_of_its_text
27. a_fail_in_a_block_is_a_user_error
28. uuid_formatting_is_total_but_storage_checks_its_domain
29. a_block_returns_its_result

## Proposal

Move each onto a script under crates/adtb-sim/scripts/AREA/ with the refusal compared whole, delete the Rust copy, and name the kept coverage per test. A test needing a file fixture or a directive the runner lacks stays in Rust and is listed.

## Open questions
