---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 61
summary: worker instructions require editing and background tools absent from other agent harnesses
closed: 2026-10-27
tags:
  - tooling
  - devex
---

# Worker instructions require harness-specific tools

## Report

`.agents/skills/lead/code-agent.md:86-89` requires named Read and Edit tools and says Edit refuses files read through sed. Lines 124-126 require run_in_background and automatic completion notification. This Codex session exposes shell, apply_patch and explicit session waiting instead, so the documented procedure cannot be followed literally.

Describe the required outcomes (bounded reads, safe edits, asynchronous execution and completion checks) independently of tool names. Keep any harness-specific examples conditional. Preserve suite exit-status and output collection requirements. Verify that a Codex worker can follow the instructions using its available tools. This is distinct from the suite serialization implemented in suite-runs-contend.md.

## 2026-10-27: resolved

Landed in `633e9a41`: portable tool guidance, authoritative root and campaign
check ownership, exit-status-preserving logs, unresolved timeout reporting,
and checkpoints based on active session capacity. The exact logging example
preserved exits 0 and 23 under Bash and Zsh; house-check passed.

## 2026-10-27: resolved
