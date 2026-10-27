---
created-at: 2026-10-26
status: open
kind: feature
priority: 81
summary: "the full check's cargo build --all-targets compiles every test a second time in the dev profile, which nothing runs"
tags:
  - tooling
  - devex
---

# Feature: a full check that compiles each test once

## Report

Since `[profile.test]` set `opt-level = 1` and `debug = "line-tables-only"`, `cargo build --workspace --all-targets` and `cargo nextest run` build different artifacts. `cargo build --all-targets` compiles test targets in the dev profile (opt-level 0, full debuginfo); nextest compiles them again in the test profile. Checked on `adtb-sys`:

```
cargo build -p adtb-sys --all-targets --message-format=json | jq -c 'select(.reason=="compiler-artifact") | {k:.target.kind, test:.profile.test, opt:.profile.opt_level, dbg:.profile.debuginfo}'
```

Actual: the lib test unit is built with `opt "0"` and `dbg 2`. Expected: the full check builds only what it runs.

The brief's full check and devenv's `check` (`devenv.nix`) both start with that build. In the 2026-10-26 tracks it cost 108 to 184 seconds per final check under load (evo ran it twice, 292s), and the duplicate set is roughly half of each workspace's 15 to 22 GB `target/`. Agents iterating with `cargo build -p CRATE` then `cargo nextest run -p CRATE` compile the crate twice per edit for the same reason.

Fix: drop the build step from `check` and the brief; `cargo clippy --workspace --all-targets` reports compile errors for every target and `cargo nextest run --workspace --no-run` builds what the suite runs. Set `[profile.dev] debug = "line-tables-only"` so the dev artifacts that remain (the shell, `cargo run`) are smaller.
