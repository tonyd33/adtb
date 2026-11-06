---
created-at: 2026-11-06
status: resolved
kind: feature
priority: 300
summary: remove the adtb-mutant crate and work/tools/mutants now hand mutants are no longer required
closed: 2026-11-06
---

# Feature: Remove the hand-mutant tooling

## Request

## Proposal

## Open questions

## Report

On 2026-11-06 the maintainer cut the requirement that every new guard, branch and match arm be shown killed by a hand mutant: it took 31% of author time, and mature projects rely on review and on a bug fix's regression test failing before the fix. The tooling stays until the read-in gate track lands, since that track has mutant sites in flight.

Remove `crates/adtb-mutant`, its workspace entry and the dependency in `adtb`, `adtb-sys` and `adtb-lang`, its `deny.toml` allow entry, `work/tools/mutants`, the mutant-site rule in `house-check` and its `house-check-test` case, the `land` refusal and its `slot-test` case, and the AGENTS.md entry. Keep `pkgs.cargo-mutants` in the dev shell for the milestone exit.

## 2026-11-06: resolved

`chore: remove the hand-mutant tooling`. Removed `crates/adtb-mutant`, its workspace and crate dependencies, its `deny.toml` entry, `work/tools/mutants`, house-check's mutant-site rule and test case, `land`'s per-commit house-check loop (with the site grep gone it refused nothing) and its `slot-test` case, the AGENTS.md entry, and `check`'s touch of every source before clippy, which existed only for restored mutant edits and rebuilt everything on each check. `pkgs.cargo-mutants` stays for the milestone exit. `check --changed` on 0b8634fb: suite 3456 passed, campaign 48 passed.

## 2026-11-06: resolved
