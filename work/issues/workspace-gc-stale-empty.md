---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 91
summary: "workspace-gc reads emptiness from the last snapshot, so a workspace with unsnapshotted edits lists as empty and --clean cleans it"
closed: 2026-10-27
tags:
  - quality
  - tooling
---

# workspace-gc --clean can clean a workspace whose track is running

## Report

`workspaces` at `work/tools/workspace-gc:20-30` lists with `jj workspace list --ignore-working-copy` and decides `target.empty()` from the last snapshot. A workspace whose edits jj has not snapshotted lists as `empty`; adtb-bundles and adtb-hist, both with tracks running, list so now. `--clean` (`:53-55`) then runs `cargo clean` in it once its target has been idle two hours, which a long read or edit phase reaches.

The forgotten-workspace pass also treats any sibling `adtb-*` directory that is not a workspace as forgotten.

Fix: decide emptiness from a fresh snapshot (`jj status` in each workspace, or drop `--ignore-working-copy`), and touch only siblings that hold a `.jj`.

## 2026-10-27: done

`work/tools/workspace-gc` decides each workspace's copy state from a fresh snapshot (`jj -R PATH log -r @` without `--ignore-working-copy`); a workspace jj cannot read lists as `unknown` and is kept. The forgotten pass lists only sibling directories holding a `.jj`. Checked by hand on a scratch repo: a workspace with an unsnapshotted file lists `changed`/`keep`, an idle empty one `clean`, a forgotten one `forgotten`, and a plain sibling directory is not listed. Nothing stays open.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qtools track.
