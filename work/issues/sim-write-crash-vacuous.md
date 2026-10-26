---
created-at: 2026-10-25
status: open
kind: bug
priority: 590
summary: the write crash test skips its checks when recovery fails to open and still passes
tags:
  - quality
  - sim
  - test
---

# The write crash test passes when no seed reaches its checks

## Report

`committed_writes_survive_a_crash_and_sequences_never_reissue` (`crates/adtb-sim/tests/write.rs:1160`) skips a seed's checks on three paths: no roots recorded (`:1243`, `continue`), `Db::build` returning `None` after the crash (`:1247`, `return`), and so no recovery result (`:1257`, `continue`). `Db::build` returns `None` whenever `Pager::open` fails (`crates/adtb-sim/tests/support/mod.rs:297`, `.ok()?`), so a recovery bug that leaves the file unopenable skips the seed instead of failing it. The only coverage check, `crashed > 0` (`:1294`), counts crashes after an acknowledged commit, not seeds whose survivors were checked: every seed can skip and the test passes.

Fix: count the seeds that reached the acknowledged-rows checks and assert it over the crash count (or that every crashed seed with acknowledged commits reached them); fail when the pager does not reopen after roots were recorded, since the roots are taken after `flush_log`.
