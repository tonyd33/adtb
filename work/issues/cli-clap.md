---
created-at: 2026-10-29
status: resolved
kind: feature
priority: 450
summary: Parse the command-line arguments of every binary with clap
closed: 2026-10-29
---

# Feature: Parse command-line arguments with clap

## Request

Five binaries parse their arguments by hand: crates/adtb-shell/src/main.rs,
crates/adtb-probe/src/main.rs, crates/adtb-codegen/src/main.rs,
crates/adtb-conformance/src/main.rs and apps/campaign/src/main.rs. Each has its own help text,
missing-value errors and unknown-flag handling.

## Proposal

Lead ruling 2026-10-29: use clap with the derive API in all five.

- `clap = { version = "4", default-features = false, features = ["std", "derive", "help",
  "usage", "error-context"] }` as a workspace dependency; the campaign workspace declares its
  own.
- Each new crate gets a deny.toml `[bans].allow` entry with its justification.
- Every existing flag, positional and exit code stays. Help and error wording may change.
  Check docs/manual/, docs/development.md and work/tools/ for each invocation.
- Delete each hand-rolled `Args::parse` and its tests of the parser itself; keep tests of
  behaviour.

## Open questions

## 2026-10-29: done

adtb, adtb-probe, adtb-codegen, adtb-conformance and campaign parse with clap; flags and exit codes kept. Accepted changes: `--` is accepted, `adtb -` is a FILE, an empty path exits 2, adtb-conformance refuses an unknown flag. Branch check 2869 passed, 7 skipped, campaign 52.

## 2026-10-29: resolved
