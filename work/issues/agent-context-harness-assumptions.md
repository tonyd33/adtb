---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 66
summary: agent guidance hardcodes a 200k checkpoint threshold and harness-specific LSP and output behavior
closed: 2026-10-27
tags:
  - tooling
  - devex
---

# Context guidance assumes one harness capacity and tool behavior

## Report

`.agents/skills/lead/SKILL.md:76-78` says to checkpoint after about 200k tokens. `.agents/skills/lead/code-agent.md:24-35` states fixed LSP workspace scope and output-storage behavior. These properties depend on the harness and configuration; a session may compact before the stated checkpoint or have different symbol scope and output limits.

Base checkpointing on the active session capacity and meaningful task boundaries. State LSP scope and output behavior as conditional guidance to verify, with search and bounded shell reads as fallbacks. Preserve the existing goal of small reads and durable handoffs without assuming a universal token budget or tool implementation.

## 2026-10-27: resolved

Landed in `633e9a41`: portable tool guidance, authoritative root and campaign
check ownership, exit-status-preserving logs, unresolved timeout reporting,
and checkpoints based on active session capacity. The exact logging example
preserved exits 0 and 23 under Bash and Zsh; house-check passed.

## 2026-10-27: resolved
