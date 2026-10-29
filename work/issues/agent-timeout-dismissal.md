---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 64
summary: worker guidance treats a suite timeout followed by an isolated pass as proven load
closed: 2026-10-27
tags:
  - tooling
  - devex
---

# Worker guidance dismisses unresolved test timeouts

## Report

`.agents/skills/lead/code-agent.md:126-127` says a test that times out in the full run and passes alone is load, not a failure. An isolated pass does not establish the cause: intermittent concurrency bugs can also disappear in isolation. Existing load measurements in suite-runs-contend.md do not prove the cause of future timeouts.

Require the worker to report the timed-out test, suite conditions and isolated outcome, preserving the unresolved cause unless evidence establishes it. Keep the policy against unnecessary repeated full suites, but do not turn an isolated pass into a clean full-suite result.

## 2026-10-27: resolved

Landed in `633e9a41`: portable tool guidance, authoritative root and campaign
check ownership, exit-status-preserving logs, unresolved timeout reporting,
and checkpoints based on active session capacity. The exact logging example
preserved exits 0 and 23 under Bash and Zsh; house-check passed.

## 2026-10-27: resolved
