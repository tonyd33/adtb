---
created-at: 2026-10-26
status: resolved
kind: feature
priority: 704
summary: "check never builds or tests apps/campaign, and its manifest copies the root lints but not the test profile"
closed: 2026-10-26
tags:
  - quality
  - tooling
  - test
---

# Feature: run the campaign manager's tests and clippy in check

## Report

`apps/campaign` builds and passes its 21 tests in isolation, but nothing runs it: the root `Cargo.toml` has `exclude = ["apps"]`, the devenv `check` (`devenv.nix:21-28`) runs only `--workspace` build, clippy and nextest, and `.config/nextest.toml` does not name it. An engine or language change can break the app silently; `campaign-release-1-2-refused-by-engine.md` is a live case. Its `Cargo.toml` copies the root lint table by hand but not the root `[profile.test]` (`opt-level = 1`, `debug = "line-tables-only"`), so its tests run at opt-level 0, and it keeps its own lock file and target directory, rebuilding adtb on every run.

Fix: add `cargo clippy` and `cargo nextest` with `--manifest-path apps/campaign/Cargo.toml`, sharing `CARGO_TARGET_DIR`, to `check`, and take the lints and test profile from the root so they cannot drift.

## 2026-10-26: no slow timeout

With no `.config/nextest.toml` of its own and agents running `cargo test` there, a test that deadlocks runs until the shell's 600 second cap: the app track lost 10 minutes to one hung `engine` test before killing it by hand. Giving the app the root's nextest profile (or running it from the root config) bounds it.

## 2026-10-26: done

`check` (`devenv.nix`) sets `CARGO_TARGET_DIR` to the root `target` unless already set, then builds, lints with `-D warnings`, checks formatting and runs nextest for `apps/campaign/Cargo.toml` with the root `.config/nextest.toml`. The app's manifest gains the root `[profile.test]`, and `check` fails when its text from `[workspace.lints.rust]` to the end differs from the root manifest's, so the lints and profile cannot drift. The app keeps its own workspace and lock file.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed with the cat track.
