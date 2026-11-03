---
created-at: 2026-11-03
status: open
kind: bug
priority: 620
summary: "create leftovers: a marker removal races an open of an unlinked file, a symlink swap races the removal, publish's lock scope is untested, and several refusals read oddly"
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

## 2026-11-03: leftover marker landed

`fix: remove create's leftover marker on the next whole open`. A successful open of an existing file calls `remove_leftover` (crates/adtb/src/storage/create.rs): it removes `NAME.creating/` only when its one entry is a whole marker naming the file's id, decided by `owner`, which create's `settle` shares; any failure leaves it and the open succeeds. storage.md §5.3 states it. `publish` now holds the segments guard across both renames and the `segments.dir` assignment; the race was unreachable, since nothing runs beside create's publish. Tests in crates/adtb-sim/tests/create.rs: the crash search requires no reserved file or directory after every stage's next open; a marker of another id, a torn marker, an extra entry, the aside file and an aside log segment beside the file's own marker, and on the host a symlinked marker or directory, each left byte for byte; the owned branch of `refuse_foreign_log` rolls forward. Six mutants fail by name. Class: multi-step change not atomic under failure, crash or race. Full check 3633 passed. Priority 330 to 620: what is left loses no data.

Left, besides the items above not in that track (`Held::published`'s `unwrap_or(true)`, `FILE-log` as a regular file, an open racing another process's create, the `NoHeader` Debug name, refusing at create's start, `Unpublished::discard` before the log directory exists):

- With NAME unlinked while open and a new create of NAME by another process, `remove_leftover` acting on its listing can unlink that create's marker; the create then fails with an I/O error or answers `Reserved` on retry. No lock serialises them, since the aside file does not exist yet. `settle` has the same window.
- The reserved directory can be swapped for a symlink between `Held::hold`'s check and the remove (a hostile writer in the parent directory); `settle` has the same window.
- No test fails if `publish` stops holding the segments guard across the renames.
