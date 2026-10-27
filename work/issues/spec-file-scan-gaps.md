---
created-at: 2026-10-26
status: open
kind: feature
priority: 287
summary: "interop.md leaves open a missing literal path, record numbering under a glob, Text in JSON, value-level sum header columns, non-UTF-8 files and conformance file fixtures"
tags:
  - spec
---

# Feature: File scan gaps in interop.md

## Report

Found by the reference file scans track; the reference's current choice follows each item:
- A literal path naming no file has no stated outcome. The reference gives `BadInput` with line 0; a glob matching nothing is the empty set.
- Under a glob, the rejects key `(load, line)` collides if lines restart per file. The reference numbers records across files in name order.
- The JSON mapping says every other scalar is a string holding its textual syntax, but the §2.4 example writes Text as a plain string. The reference follows the example.
- Where the header fields of a value-level sum go among its tag columns is unstated. The reference puts them at `f.h`.
- A file that is not UTF-8 is unspecified. The reference fails the whole scan with `BadInput`, line 0, even with a rejects relation.
- A conformance script cannot carry its own files. The runner resolves paths against the script's directory.

## 2026-10-26: line endings, blank lines and a byte order mark

interop.md's CSV row says only "RFC 4180", which ends a record at CRLF. The engine and the reference now also end a record at LF and at a bare CR, skip empty records (a line holding only spaces is still a record) and number records without them, and strip one leading U+FEFF from a CSV or NDJSON file. The spec should state each of these or refuse them.
