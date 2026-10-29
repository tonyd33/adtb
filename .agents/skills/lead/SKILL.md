---
name: lead
description: Lead adtb work as maintainer across several agents. Split work into tracks, brief and dispatch one fresh agent per track in its own jj workspace, rule on what agents raise, review and land what comes back, and keep tickets and the quality cadence current. Use when dispatching or landing a track, coordinating parallel agents, or resuming lead work from the handoff. Not for implementing a track yourself.
---

# Lead

Keep your context for rulings, briefs and review. Agents write the code.

## Plan

- Choose work from `tix list --ready`. A design question goes to the
  design space, not to a track.
- Give each agent one track, in its own workspace beside the main
  checkout: `jj workspace add --name NAME ../adtb-NAME`.
- Before dispatch, confirm the worker can write that workspace and access
  the shared jj metadata under the active harness's permissions. Workspace
  creation alone does not grant worker access. Obtain required approval or
  provision access before implementation; report permission wait as blocked,
  not as a tool failure.
- Before starting a track, list the files the running tracks change. A
  track that changes a shared signature (pager, heap or B-link tree calls)
  does not run beside tracks that call it: land it first and branch the
  others after, or state the landing order in every affected brief.

## Brief

- Write the brief to `work/briefs/brief-NAME.md` in the main checkout.
  jj ignores that directory; earlier briefs there, if any, are templates.
  Keep essential scope, acceptance commands and landing dependencies in a
  tracked ticket or supporting note using [the evidence workflow](verification.md).
  The local brief must not be the only copy needed to resume.
- Tell the agent to read [code-agent.md](code-agent.md) and its brief,
  both by absolute path.
- A brief names the rulings it relies on, the design and spec sections by
  heading and line range, the tickets, the acceptance tests, the crates
  it may add, a short map of the crates it touches, and the landing order
  when tracks overlap.
- Mark the track's ticket `in-progress` with `branch` set to the
  workspace name.

## While tracks run

- Rule on the design questions agents raise. Settle a hard call yourself
  in a proposed point with your argument as its reason, and present it to
  the user for confirmation. Do not hand the question back open.
- Treat every spec change an implementation asks for as suspect. Name the
  tier it belongs to before accepting it.
- File the gaps agents report as tickets.
- Once an agent has reported, message it again only when it must act.
  Save each report's handoff note and verification receipts to
  `work/briefs/handoff-NAME.md`; update the durable task state with unresolved
  failures, dependencies and the next action. Follow-up work goes to a new
  agent whose brief points at that note.
  Fix trivial rebase fallout yourself.

## Land

Review architecture as well as behavior. Before accepting custom orchestration,
compare it with the underlying tool's native configuration and extension points.
Each wrapper, watcher and staging layer needs a concrete unmet requirement.
Ask a fresh reviewer to challenge those layers and their responsibility placement.
Passing tests do not justify unnecessary machinery.

1. Review the workspace's diff against its brief and the rulings, and run
   `work/tools/house-check ../adtb-NAME`.
2. Run `work/tools/land NAME 'SUBJECT'` with the workspace name, not its
   directory. Propose SUBJECT to the user first unless they allowed
   commits without it. It snapshots the workspace, refuses while the
   main working copy has changes, rebases the track onto the main
   checkout's parent, moves `main` and forgets the workspace. Agents that file tickets write into the main working copy
   at any time, so when `land` refuses, describe those changes and run
   `jj new`, then run it again.
3. When `land` reports conflicts, send the track back to its agent with
   the commits that landed since its base.
4. Run `check`. Fix trivial failures yourself; ticket the rest for a new
   track. Close the ticket with a dated section. Remove
   `../adtb-NAME` once nothing in it is needed.

When `check` refuses under the free-space guard, ask the user before
deleting anything. First list the candidates without deleting: each
`target/debug` or `target/release` of a forgotten workspace that holds a
`.fingerprint` directory, with its size. Then delete each one by its
literal absolute path in its own command. Never `rm -rf` a path built in a
loop or from a variable. Keep `target/tmp` and every file at the top of
`target/`: they hold test data and verification evidence.

## Quality cadence

- After every 10 to 15 landed code commits past the `reviewed` bookmark
  (`jj log -r 'reviewed..main'`), dispatch a fresh agent on the
  [quality-review workflow](../quality-review/SKILL.md). After reading its
  report, run `jj bookmark set reviewed -r main`.
- Spend about 15% of each milestone on one track over tickets tagged
  `quality`.
- Run cargo-mutants and llvm-cov only at a milestone exit.

## Context

Checkpoint at meaningful task boundaries and before the active session
approaches its context limit or compaction threshold. Use the capacity
and warnings the harness exposes; do not assume a fixed token budget.
Follow [the evidence workflow](verification.md) for receipts and durable
state. Keep `work/HANDOFF.md` short and current, archiving superseded
narrative without losing unfinished work. On resume, verify the live state;
historical permissions in a handoff are not authorization for a new session.
