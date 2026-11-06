---
created-at: 2026-11-06
status: open
kind: feature
priority: 300
summary: remove the adtb-mutant crate and work/tools/mutants now hand mutants are no longer required
---

# Feature: Remove the hand-mutant tooling

## Request

## Proposal

## Open questions

## Report

On 2026-11-06 the maintainer cut the requirement that every new guard, branch and match arm be shown killed by a hand mutant: it took 31% of author time, and mature projects rely on review and on a bug fix's regression test failing before the fix. The tooling stays until the read-in gate track lands, since that track has mutant sites in flight.

Remove `crates/adtb-mutant`, its workspace entry and the dependency in `adtb`, `adtb-sys` and `adtb-lang`, its `deny.toml` allow entry, `work/tools/mutants`, the mutant-site rule in `house-check` and its `house-check-test` case, the `land` refusal and its `slot-test` case, and the AGENTS.md entry. Keep `pkgs.cargo-mutants` in the dev shell for the milestone exit.
