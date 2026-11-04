---
created-at: 2026-11-03
status: open
kind: bug
priority: 520
summary: "the simulator does not model renaming a directory into its own child, so failure searches skip renames out of *.creating/"
---

# Simulator models renames out of a creating directory

## Report

Found in S5.1 of dump-restore-backup. The create path undoes a failed create by renaming the log directory out of `NAME.creating/`; the simulator does not model that rename, so the dump and restore failure searches skip renames out of `*.creating/`. Fix: model it in adtb-sim's file system, then drop the skip from crates/adtb-sim/tests/dump.rs and any backup search that has one.
