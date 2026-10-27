---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 764
summary: a bare carriage return in a CSV file fails the whole file with a message about a quoted cell
closed: 2026-10-26
tags:
  - quality
  - engine
---

# A bare carriage return in a CSV file is reported as a quoted cell ending early

## Report

A carriage return not followed by a line feed after an unquoted cell (old Mac line endings, or a CR inside an unquoted cell) falls into the quoted-cell branch of `split` (`crates/adtb-file/src/csv.rs:83-86`), and the whole file fails at line 0.

The file `id,name\r1,ann\r`:

Actual: `BadInput` line 0 `byte 7: a quoted cell ends before its separator`.

Expected: either the file reads as two records, or the message names the bare carriage return and its line.

Fix: give the bare CR its own case in `split`, decide whether it separates records, and test it.

## 2026-10-26: done

A bare carriage return now ends a record in `split` (`crates/adtb-file/src/csv.rs`), as a line feed and CRLF do, so `id,name\r1,ann\r` reads two rows. The quoted-cell message is left only for a quoted cell followed by something other than a separator. Tests: `a_bare_carriage_return_ends_a_csv_record` in both `files.rs`. interop.md says RFC 4180, which names only CRLF; accepting LF and CR is a spec gap for `spec-file-scan-gaps.md`.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed with the fbug track.
