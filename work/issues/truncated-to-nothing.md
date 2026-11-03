---
created-at: 2026-10-30
status: resolved
closed: 2026-11-03
kind: bug
priority: 428
summary: "a file cut to 0 bytes with create set opens a fresh empty database beside the old log, though a file with neither header copy does not open"
---

# Truncated to nothing

## Report

The seeded file of crates/adtb-sim/tests/truncated_open.rs cut to 0 bytes with its log kept. catalog.md 11.2 says a file with neither header copy does not open. A crash during create leaves an empty file too, so tell the two apart by whether the log holds anything past the create.

## 2026-11-03: landed

Closed by `fix: refuse to open an empty file beside a log no create wrote`. After create-aside, an open with create over an empty file beside a foreign log already refused at publish, `LogPresent`, nothing changed, after building a whole database aside (287 ms simulated against 3 ms). Without create it refused as `Corrupt "NoHeader"`, never naming the log. Now an open without create of an empty file checks log ownership (`refuse_foreign_log` in crates/adtb/src/storage/create.rs) and refuses `LogPresent`. crates/adtb-sim/tests/truncated_open.rs sweeps cut lengths in both modes, every byte and directory compared. Check exit 0, 3626 passed.

Left, from the review, carried to create-leftover-marker:
- The owned branch of `refuse_foreign_log` has no test: an empty file beside create's own published log (a crash between create's renames over an empty file) must answer `NoHeader` without create and roll forward with it; mutating `if owned` away passes every test.
- `FILE-log` as a regular file with a whole marker answers `Io(NotADirectory)`, not `LogPresent`; untested.
- An open racing another process's create can answer `LogPresent` where a retry succeeds.
- A file with no whole header reads `NoHeader` (a Debug name) without create and "neither copy of the file header verifies" with create; truncated_open.rs and adtb-shell/tests/shell.rs pin the first.
- Create still builds a whole database before refusing at publish; refusing at its start shifted fixed seeds in server_binding and vacuum floors (picked-seed-floors).
