---
created-at: 2026-10-30
status: resolved
kind: feature
priority: 355
summary: Measure where a track spends its time and cut it
closed: 2026-10-30
---

# Feature: Agent wall time

## Request

Tony, 2026-10-30: see if we can speed up agent work. Research in work/briefs/brief-devex.md, report in review-devex.md.

## Proposal

## Open questions

## 2026-10-30: the manual test

adtb-shell's every_page_prints_what_it_shows takes 36.6 s alone at load 15 and timed out at 60 s
at load 25 in a branch check. It now has a 180 s limit. Find out why it takes that long.

## 2026-10-30: rulings from the measurements

Share of an agent's own time over 4 tracks: suite build 24%, targeted nextest 22%, model turns 23%, mutation 13%, suite run 6%, other builds 6%, reading 1%, suite lock wait 0%. A fresh workspace's first suite build takes 431 to 574 s; a warm edit takes 5 to 15 s for one binary and 38 to 92 s for the workspace.

Adopted now, in the lead skill and code-agent.md: at most 4 building tracks, delta review after blockers, full check on the final round, one suite per round, none after a clean rebase, and mutants run by name. Not adopted: a cap on mutants, because mutation found real survivors in both recent reviews.

Next, as one track after test-time-engine's profile: `debug = 0` in dev and test; `land` keeps workspace directories as reusable slots so target/ stays valid, with a size cap; the `check` lock of check-machine-lock.

Rejected, measured: one adtb-sim binary (editing a sim test goes from 1 s to 60-83 s), `opt-level = 0` for local crates (suite 358 s vs 112 s), dropping the suite lock (150 s vs 112 s), a smaller suite (`quick` is no faster).

## 2026-10-30: landed

Slots keep a build directory across tracks, `check` and `suite` share one machine lock, and tests build with no debug info. The rejected options and their measurements are in the lead skill and work/kb/cargo-disk.md.

## 2026-10-30: resolved
