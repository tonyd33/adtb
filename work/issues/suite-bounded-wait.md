---
created-at: 2026-11-02
status: open
kind: feature
priority: 380
summary: agents poll suite and check logs because neither can be started detached and waited on for a bounded time
tags:
  - devex
---

# Feature: Suite bounded wait

## Report

`work/tools/suite` and `check` run in the foreground until done. With the machine lock queue and a full run, that outlasts a harness's foreground limit, so agents run them in the background and poll the log: in a 14 day measure, foreground `until grep`, `tail -F` and `sleep` loops on these logs took 5.7 agent hours in Claude Code subagents, and 498 short `write_stdin` polls took 11% of Codex subagent time. code-agent.md now says how to wait in each harness; the tools still give no way to wait that is the same everywhere.

## Proposal

`suite --start [ARGS]` and `check --start` take a run id, detach the run under the machine lock, and print the id and log path at once. `suite --wait ID [SECONDS]` blocks until the run ends or SECONDS pass, default under every harness's foreground limit: on the end it prints failures, the summary and `suite: exit N` and exits N; on the limit it prints `still running` and exits with a distinct status, so a caller repeats the one call. No loop in the caller, no log reading. A detached run outliving its starter keeps the rule that a child outliving the shell that took the lock runs unlocked: decide whether `--start` owns the lock itself instead.

Done when `slot-test` or `suite-test` proves start, wait to the end, wait to the limit and a second waiter on one run, and code-agent.md's per-harness lines name the one call.
