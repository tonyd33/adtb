---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 694
summary: "a blank line in a CSV file reads as a record of one empty cell, while NDJSON skips blank lines"
closed: 2026-10-26
tags:
  - quality
  - engine
---

# A blank line in a CSV file reads as a row of one empty cell

## Report

`split` (`crates/adtb-file/src/csv.rs:66-90`) turns an empty line into a record holding one empty unquoted cell. With a one-column header a trailing blank line adds a phantom row; with more columns it is a `BadInput`, or a reject row under `rejects into`. NDJSON skips blank lines (`crates/adtb-file/src/json.rs:41`), so the two formats disagree.

The file `name\nann\n\n` read at `{ name: Text }`:

Actual: `["", "ann"]`.

Expected: `["ann"]`.

Fix: skip blank lines as the NDJSON reader does, add a test for each layout, and add the rule to `spec-file-scan-gaps.md` if the spec does not state it.

## 2026-10-26: done

`split` in `crates/adtb-file/src/csv.rs` drops every empty record, before the header as well as after it, so a blank line is no record in CSV as in NDJSON, and records are numbered without blank lines. A line holding only spaces is still a record. Shared by the engine and the reference. Tests: `a_blank_line_in_a_csv_file_is_no_record` and `records_are_numbered_without_the_blank_lines` in `crates/adtb-reference/tests/files.rs`, `a_blank_line_in_a_csv_file_is_no_record` in `crates/adtb-sim/tests/files.rs`. interop.md is silent on blank lines; the rule still needs a line in `spec-file-scan-gaps.md`.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed with the fbug track.
