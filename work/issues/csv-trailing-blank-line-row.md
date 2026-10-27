---
created-at: 2026-10-26
status: open
kind: bug
priority: 694
summary: "a blank line in a CSV file reads as a record of one empty cell, while NDJSON skips blank lines"
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
