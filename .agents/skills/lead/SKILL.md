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
- Before each full `check`, run `work/tools/workspace-gc --guard`; it
  fails under 30 GB free and lists the build directories. Nothing else
  checks free disk.
- Run at most one full `check` on the machine at a time. `suite` locks
  only the tests, and two concurrent checks have exhausted the machine.
  While a check runs, tell agents to build with `CARGO_BUILD_JOBS=6`.
- Read a check's own exit status before landing, and never land in the same command
  as the wrapper that ran it: a wrapper's last command hides the failure. `check` runs
  the main checkout's dev shell, so a branch older than a tool main added exits 127.
  Rebase the track onto main first, then run its check.
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
- Name every report, receipt and handoff path a brief asks for so it does
  not exist yet: add a round suffix (`review-NAME2.md`). An agent's Write
  replaces an ignored file with no copy left.
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
   `work/tools/house-check ../adtb-NAME --from BASE --to NAME@` with the track's base.
   Without a range it checks only the working copy, which is empty once the agent commits.
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
5. Once the main check passes, delete the landed workspace's
   `target/debug` and `target/tmp`, each by its literal absolute path in
   its own command. They hold build output and test scratch, about 22 GB
   per workspace. Keep the other files and directories under `target/`.
6. Read what is about to be published before every push, tickets and
   `work/` included: the repository is public, and a push cannot be taken
   back. Run `jj diff -r 'main@origin..main'` and take out credentials,
   personal data and machine details. Move them to `work/briefs/`, or
   leave them out.
7. Push `main` to GitHub: `jj git push --remote origin --bookmark main`.
   `origin` is git@github.com:tonyd33/adtb.git; `tangled` is the old
   remote and is no longer pushed.

Any other deletion needs the user's approval. When `workspace-gc --guard`
fails, first list the candidates without deleting: the
`target/debug`, `target/release` and `target/tmp` of each forgotten
workspace, with sizes. Ask, then delete each one by its literal absolute
path in its own command. Never `rm -rf` a path built in a loop or from a
variable. Keep every other file and directory under `target/`: they hold
verification evidence.

## Quality cadence

- After every 10 to 15 landed code commits past the `reviewed` bookmark,
  counted by `work/tools/code-commits | wc -l` (commits that change only
  docs/, work/ or Markdown files do not count), dispatch a fresh agent on the
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
