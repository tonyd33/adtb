---
created-at: 2026-11-03
status: open
kind: bug
priority: 330
summary: "A crash after create's commit point can leave NAME.creating/id beside a whole database, and nothing removes it"
---

# Create leftover marker

## Report

Found by the create-aside-2 crash search. Create publishes by renaming the file out of `NAME.creating/`, which is the commit point, then removes the marker `NAME.creating/id` and the directory. A crash between the rename and those removals leaves the directory holding only a whole marker beside a whole database. Every later open finds the file and never reaches create, so the directory stays. It holds no user data and blocks nothing; the crash search in crates/adtb-sim/tests/create.rs allows exactly this residue at stage Published. A fix would remove a marker-only reserved directory under the same ownership rule create uses, from a path a whole open reaches.

From the create-aside-2 review, not fixed in that track:

- `Unpublished::publish` points the pager's log at its own name by setting `segments.dir` after the rename, outside the segments lock that the rename ran under. A segment the pager opens between the rename and that assignment is looked up under the reserved path.
- `Held::published` treats a failed metadata read as the path naming the reserved file (`unwrap_or(true)`), so it keeps the file rather than risk removing a published one, and that choice is unstated.
- When `Pager::format_new` fails before the log directory exists, `Unpublished::discard` finds no log directory in the reserved directory, takes the build as published, and leaves the marker and the file. The next create removes them, since the marker is whole.

## 2026-11-03: from the truncated-to-nothing review

- The owned branch of `refuse_foreign_log` has no test: an empty file beside create's own published log (a crash between create's renames over an empty file) must answer `NoHeader` without create and roll forward with it; mutating `if owned` away passes every test.
- `FILE-log` as a regular file with a whole marker answers `Io(NotADirectory)`, not `LogPresent`; untested.
- An open racing another process's create can answer `LogPresent` where a retry succeeds.
- A file with no whole header reads `NoHeader` (a Debug name) without create and "neither copy of the file header verifies" with create; truncated_open.rs and adtb-shell/tests/shell.rs pin the first.
- Create still builds a whole database before refusing at publish; refusing at its start shifted fixed seeds in server_binding and vacuum floors (picked-seed-floors).
