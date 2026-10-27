---
created-at: 2026-10-27
status: open
kind: bug
priority: 875
summary: "after a leading byte order mark, CSV parse errors report a byte offset 3 lower than the file's"
tags: [quality]
---

# CSV errors in a file with a byte order mark point three bytes early

## Report

`crates/adtb-file/src/lib.rs:216` strips a leading U+FEFF before `read`, and `crates/adtb-file/src/csv.rs:86` reports `byte {at}` against the stripped text, so every offset is 3 lower than in the file.

Fix: add the stripped length to the offset, or strip inside `split` and count from the file start. Pin it with a test that has a BOM and a malformed record.
