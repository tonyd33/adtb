---
created-at: 2026-10-27
status: open
kind: bug
priority: 91
summary: "workspace-gc reads emptiness from the last snapshot, so a workspace with unsnapshotted edits lists as empty and --clean cleans it"
tags: [quality, tooling]
---

# workspace-gc --clean can clean a workspace whose track is running

## Report

`workspaces` at `work/tools/workspace-gc:20-30` lists with `jj workspace list --ignore-working-copy` and decides `target.empty()` from the last snapshot. A workspace whose edits jj has not snapshotted lists as `empty`; adtb-bundles and adtb-hist, both with tracks running, list so now. `--clean` (`:53-55`) then runs `cargo clean` in it once its target has been idle two hours, which a long read or edit phase reaches.

The forgotten-workspace pass also treats any sibling `adtb-*` directory that is not a workspace as forgotten.

Fix: decide emptiness from a fresh snapshot (`jj status` in each workspace, or drop `--ignore-working-copy`), and touch only siblings that hold a `.jj`.
