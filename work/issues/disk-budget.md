---
created-at: 2026-10-29
status: resolved
kind: feature
priority: 350
summary: Cut the disk each agent workspace uses so parallel tracks fit
closed: 2026-10-30
---

# Feature: Cut the disk each agent workspace uses

## Request

Tony, 2026-10-29: parallel tracks keep running the disk under the 30 GB guard. One active
workspace target/ is 13 to 21 GB (about half `debug/incremental`), the main checkout 35 GB, and
forgotten workspaces keep fixture directories (`authority-fixtures` 1.6 GB, `sequence-regressions`
1.1 GB). Research and measure ways to cut this, then propose the changes.
## Proposal

## Open questions

## 2026-10-30: rulings from the measurements

Adopt `debug = 0` in dev and test (saves 7.5 GB per workspace; edits got faster), fixtures under target/tmp removed on success, and a `workspace-gc` flag for main's idle target/debug. Keep incremental. Rejected: incremental off, mold, sccache, a shared target dir. One adtb-sim binary is rejected by agent-wall-time's measurement. Tracked with agent-wall-time's next track.

## 2026-10-30: landed

`debug = 0` for dev and test, slots with a size cap, `workspace-gc --clear-build`, and test fixtures removed on success through `adtb_testkit::scratch::Scratch`.

## 2026-10-30: resolved
