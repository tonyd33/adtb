---
created-at: 2026-10-30
status: open
kind: bug
priority: 428
summary: "a file cut to 0 bytes with create set opens a fresh empty database beside the old log, though a file with neither header copy does not open"
---

# Truncated to nothing

## Report

The seeded file of crates/adtb-sim/tests/truncated_open.rs cut to 0 bytes with its log kept. catalog.md 11.2 says a file with neither header copy does not open. A crash during create leaves an empty file too, so tell the two apart by whether the log holds anything past the create.
