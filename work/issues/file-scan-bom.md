---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 695
summary: "a leading UTF-8 byte order mark is kept, so the first CSV header name and the first NDJSON line do not match"
closed: 2026-10-26
tags:
  - quality
  - engine
---

# A file that starts with a UTF-8 byte order mark fails to read

## Report

`records_in` (`crates/adtb-file/src/lib.rs:200`) keeps a leading byte order mark, so the first CSV header name becomes `\u{feff}id` and the first NDJSON line fails to parse. Spreadsheet exports commonly start with one.

The CSV file `\xEF\xBB\xBFid,name\n1,ann\n` read at `{ id: I64, name: Text }`:

Actual: `BadInput` with "the file has no column id".

Expected: one row, id 1, name ann.

The same mark before an NDJSON line gives `column 1: not a JSON value`.

Fix: strip one leading byte order mark per file before splitting records, and state it in the spec through `spec-file-scan-gaps.md`.

## 2026-10-26: done

`records_in` in `crates/adtb-file/src/lib.rs` strips one leading U+FEFF from each file before the format reads it, for CSV and NDJSON alike; a mark anywhere else is text. Tests: `a_leading_byte_order_mark_is_not_part_of_the_file` and `a_byte_order_mark_after_the_start_is_text` in `crates/adtb-reference/tests/files.rs`, `a_leading_byte_order_mark_is_not_part_of_the_file` in `crates/adtb-sim/tests/files.rs`. interop.md is silent on the mark; the rule still needs a line in `spec-file-scan-gaps.md`.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed with the fbug track.
