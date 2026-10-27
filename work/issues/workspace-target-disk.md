---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 82
summary: "each jj workspace keeps a 15 to 22 GB target directory, and the disk filled twice during parallel tracks"
closed: 2026-10-26
depends-on:
  - full-check-builds-dev-tests
tags:
  - tooling
  - devex
---

# Feature: bounded build directories for parallel workspaces

## Report

Every track works in its own jj workspace with its own `target/`, and `apps/campaign` keeps another inside it. On 2026-10-26 these were 15 to 22 GB each (adtb-shellux 22 GB, adtb-exec 13 GB); the speed track's grew to 66.6 GB while it compared profiles. The disk reached 100% twice: admit hit `No space left on device` writing the query cache and evo hit it creating a temp dir in `target/debug/deps`. Each then ran `cargo clean` in both build directories and rebuilt from scratch, about 3 minutes under load. Workspaces whose tracks have landed keep their `target/` until someone deletes it.

Fix:

1. A devenv script `workspace-gc` for the lead that lists every workspace of the repo (`jj workspace list`), its target size and whether its working copy is empty, and runs `cargo clean` in those that are empty or forgotten.
2. A free-space guard in the `suite` script and in `check`: below 30 GB free, print the largest target directories and stop before building.
3. Shrink what each build keeps: drop the duplicate dev-profile test build (full-check-builds-dev-tests) and build `apps/campaign` into the root `target/` (campaign-app-not-in-check).

## 2026-10-26: done

`work/tools/workspace-gc` (devenv `workspace-gc`) lists each build directory of every workspace (`target/` and `apps/campaign/target/`) and of each forgotten `adtb-*` sibling directory, with its size, whether the working copy is empty and hours since its last build; `--clean` runs `cargo clean --target-dir` on the forgotten ones and on those with an empty working copy idle for `ADTB_GC_IDLE_HOURS` (default 2); `--guard` is silent at 30 GB free (`ADTB_MIN_FREE_GB`) and otherwise prints the table and fails. `suite` and `check` start with the guard. `apps/campaign/.cargo/config.toml` builds the app into the root `target/`. Open: `cargo clean` refuses a directory without `CACHEDIR.TAG`, which the main checkout's `target/` lacks; `--clean` reports it and moves on.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed with the devex track.
