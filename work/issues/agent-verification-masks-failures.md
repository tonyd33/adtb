---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 63
summary: worker full-check pipelines hide command failures and omit campaign checks
closed: 2026-10-27
tags:
  - tooling
  - devex
---

# Worker verification can hide failed checks

## Report

`.agents/skills/lead/code-agent.md:108-117` labels a command block the full check, but pipes clippy through grep and head, and cargo deny through tail without requiring pipefail or saving the original exit status. A failed producer can therefore leave a successful pipeline status. The block also omits campaign clippy, formatting and tests that `devenv.nix` includes in check.

Make the documented final verification agree with the authoritative check command, or state explicitly which checks a worker runs and which the lead owns. Preserve original command statuses when limiting displayed output. Verify that a deliberately failing check is reported as failed and that campaign validation is assigned to an explicit owner.

## 2026-10-27: resolved

Landed in `633e9a41`: portable tool guidance, authoritative root and campaign
check ownership, exit-status-preserving logs, unresolved timeout reporting,
and checkpoints based on active session capacity. The exact logging example
preserved exits 0 and 23 under Bash and Zsh; house-check passed.

## 2026-10-27: resolved
