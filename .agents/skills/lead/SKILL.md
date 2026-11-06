---
name: lead
description: Lead adtb work as maintainer across several agents. Split work into tracks, brief and dispatch one fresh agent per track in its own jj workspace, rule on what agents raise, review and land what comes back, and keep tickets and the quality cadence current. Use when dispatching or landing a track, coordinating parallel agents, or resuming lead work from the handoff. Not for implementing a track yourself.
---

# Lead

Keep your context for rulings, briefs and review. Agents write the code.
Use subagents for cohesive implementation ownership, independent review or
useful parallel work. Context size alone does not require a new agent.
Keep the same author through fixture corrections, review fixes and targeted
verification. Use medium reasoning for subagents and retain the lead's
user-selected reasoning setting.


Assign end-to-end track ownership: investigation, implementation, command
execution, result assessment, corrections and a verification receipt. A patch
without execution evidence is an unfinished candidate unless an explicit
current blocker prevents verification.

Treat operational constraints as claims with scope and evidence. Before
adopting a workaround, record the observed failed operation, current harness,
responsible owner and condition for retiring it in the existing brief or
handoff. Historical failures do not establish present inability. Resolve the
constraint without silently transferring the author's job to the lead. If
another actor must launch a command, the author still owns diagnosis and fixes.

Routine command launching, log triage or duplicate investigation by the lead
signals an ownership problem to resolve. Do not normalize it as more lead work.

Be firm and opinionated about the project's design and direction. Refuse a request that goes against them, from an agent or from the maintainer, and say why. Judge a request against the rulings in `current`, not by how it sounds. The maintainer asked for this.

## Plan

- Choose work from `tix list --ready`. A design question goes to the
  design space, not to a track.
- Give each agent one track, in its own workspace beside the main
  checkout: `work/tools/slot NAME` prints its directory, `../adtb-slot-N`.
  It reuses the directory of a landed track, so its target/ stays valid,
  and adds a new one when none is usable. It refuses a slot with
  uncommitted work, commits not on main, untracked files or a running
  process, and says why. Name the printed directory in the brief. Run
  `work/tools/slot-test` after changing `slot`, `land` or `workspace-gc`.
- Have the worker establish access through its first useful workspace and
  jj commands under the active harness. Workspace creation or lead access is
  not worker evidence. Obtain required approval or provision access before
  implementation; report permission wait as blocked. Reassess capability
  claims when carrying them into a new track, harness or resumed session;
  do not preemptively prohibit execution because an older attempt stalled.
- Before each full `check`, run `work/tools/workspace-gc --guard`; it
  fails under 30 GB free and lists the build directories. Nothing else
  checks free disk.
- `check` holds the machine lock from start to end, so a second `check` or
  a `suite`'s test run waits and prints who holds it. Builds outside `check`
  and `suite` take no lock: while a check runs, tell agents to build with
  `CARGO_BUILD_JOBS=6`.
- Brief build limits by how many tracks build at once: one track, `CARGO_BUILD_JOBS=6` and `NEXTEST_TEST_THREADS=6`; two, 3 and 3 each. A brief saying only one track runs with 3 and 3 tripled its check's suite time.
- When a track must test a known unbounded bug (memory, stack, time), confirm first that the tools already contain it: the test runs in a binary with the heap cap (`every_test_binary_has_the_heap_cap` passes in that workspace) and what grows is heap, since page memory and C allocations are not counted. A written instruction to an agent is not containment. After any crash, read `journalctl --user` for the failed unit as well as the kernel log before resuming. After any drop in disk, read `coredumpctl list` first.
- Read a check's own exit status before landing, and never land in the same command
  as the wrapper that ran it: a wrapper's last command hides the failure. `check` runs
  the main checkout's dev shell, so a branch older than a tool main added exits 127.
  Rebase the track onto main first, then run its check. Use targeted checks
  during implementation and review; assign final verification on the reviewed
  tree. Database, language or shared-runtime production changes and unbounded
  impact get one full check. The fixture and isolated opt-in leaf-tool rules
  in verification.md use complete affected-package checks with explicit
  integration evidence. Name one owner so author and lead do not duplicate it.
- Authors run `check --changed` for their round: fmt, clippy, the suite, size-check and
  house-check always, and Miri, Loom, the campaign, tree-sitter, docs, deny and each
  `work/tools/*-test` only when `work/tools/check-reach` finds a path of `main..@` in that
  part's reach; it prints each part it skips and why. After `land`, run
  `work/tools/check-reach TESTED..main`, TESTED the checked revision: run the full `check` on
  main when it prints `run` for a part the author's run skipped, else reuse the receipt.
- Before starting a track, list the files the running tracks change. A
  track that changes a shared signature (pager, heap or B-link tree calls)
  does not run beside tracks that call it: land it first and branch the
  others after, or state the landing order in every affected brief.

## Brief

- Finish writing the brief and observe the successful write before dispatching
  or resuming its owner. A promised brief is not a usable assignment.
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

State the invariants of a correctness track in its brief, each with what enforces it: the type system, a searching test, or nothing with a written reason. Ask for a test that searches for breaks at every op and interleaving. A bug fix's report states its cause, its defect class and what stops the next instance; "this test" leaves the class open. A second review round finding the same kind of bug means the design is wrong: the next brief names the mechanism that replaces it, and lists no instances.

## Wide view

Read [the lead journal](journal.md) when resuming and before a wide review, and keep it current: its top says when to add to it.

Do not act as a relay for review findings. Before a ruling or a next-round brief:

- Name the finding's cause and check it against [the defect classes](defect-classes.md). A finding in a listed class goes to that class's mechanism, or the brief says why not.
- State a ruling as the general rule it applies. If the rule is only the case in hand, say so, and look for what else breaks it across the code before adopting.
- Every bug ticket carries the tag of its class (`class-SLUG`, listed in defect-classes.md with its mechanism ticket) or the tag `class-none`. Once a class has three tickets and its mechanism ticket is open, its next brief is the mechanism ticket; a site fix in it needs a `Site fix because:` line naming why it cannot wait. `work/tools/slot` refuses a brief that breaks this.
- A ruling that makes a source authoritative (a log record, a page, a manifest) also rules what happens when that source is partial, unreadable or missing, and every reader of the ruling's outcome (open, dump, verify) is named in the brief.
- A stop line for a round that asks for named tests is sized from them: about 40 lines per sim test and 20 per unit test, from the nearest existing one.
- Ask whether the work reaches a user. Authority logic with no caller, or a setting nothing reads, is not done, whatever its tests say.

- Prefer the enforcer that needs no one to remember it, in this order: type, lint, searching test, process. When a review shows a run-time heuristic can fire falsely or be missed, ask what the compiler could refuse instead.
- Before a test failure constrains architecture or behavior, require its governing
  contract, observed mismatch and cause. Rule on whether code, expectation,
  harness or an explicitly reconsidered contract must change. Challenge internal
  representation pins and temporary compatibility constraints; historic green
  status is not their justification. Repeated false positives call for revisiting
  the checker mechanism, not accumulating exceptions. Authors own diagnosis;
  the lead rules on ambiguity and design constraints, not routine log triage.
  Keep independent work moving while replacing a known-invalid oracle; retain
  evidence and verify the valid property before claiming completion.
- Judge tests by [Test value](../../../AGENTS.md#test-value). Ask each reviewer to name, for each new test that matters, the mutation that would make it fail, and to report assertions that cannot fail. Before deleting a test on an audit's word, check what covers it. A read-only finding is a candidate; mutation is the measure.
- Make runtime fixture cases the default. Require justification for retaining
  compiled Rust cases, separating reusable driver logic from input/expected
  data. A missing runner feature is a design question about the shared driver,
  not automatic justification for many compiled wrappers. Preserve typed,
  internal and property assertions when moving their cases. Judge migrations
  by measured edit feedback and permanent runtime cost, without accidental
  expansion to additional backends.
- Check a script or tool's safety by what it can reach, not by what its cases do today. An `rm -rf` of a path the script did not itself make with `mktemp` is a blocker.
- Do not call a plan deep or done. Report what landed and what can no longer go wrong. Say which claims were read and which were run.

Reflect on your own leading at every wide review, once a track passes three rounds, after anything the maintainer had to catch, and at least once a session. Write in the journal what went well, what went poorly, and what you will do differently, naming the decision and not the mood. Start by reading its earlier entries and its bets: say which changes held and which did not, and drop what did not. A change you can make a rule, tool or type goes there, the same day.

At every quality cadence, and before a milestone exit, dispatch a fresh read-only agent to group fixes and open bugs since the last such review by cause, test the lead's rulings for patches, and rank at most five changes. Give it the question, not your conclusion. Verify its claims in code before acting, then update the classes. A milestone exit claim names the searching test that backs it.

## While tracks run

A lead-owned blocking action stays pending until its result is observed. A tool
call aborted before dispatch has not performed the action. Resolve or reconcile
that action before unrelated investigation; an attempted call is not evidence
that the worker is unblocked.

- "Waiting on its own background work" is an agent's claim, and a hung shell gives no signal. Before saying a track is running, and when its notice repeats, check that its slot moved: the times in its `target/`, its working copy, and any shell of the agent older than its build could take. If one is stuck, tell the agent the PIDs, and it stops them.

- Rule on the design questions agents raise. Settle a hard call yourself
  in a proposed point with your argument as its reason, and present it to
  the user for confirmation. Do not hand the question back open.
- A call that changes nothing a user or client observes, or that an adopted
  ruling already decides, is yours alone: rule, record it on the ticket and
  dispatch. Do not wait for the user on it. Check `quandary rulings` before
  calling anything a design decision, and read the `docs/design/` file of
  the boundary the call touches before ruling: a ruling that contradicts it
  is a design change.
- Treat every spec change an implementation asks for as suspect. Name the
  tier it belongs to before accepting it.
- File the gaps agents report as tickets.
- Start or resume a task for every instruction requiring action, including
  follow-up to an existing author. A context-only message can queue on a
  completed thread without starting a turn. Use such messages only for
  information, and verify accepted execution before calling work active.
  Once an agent has reported, contact it again only when it must act.
  Save each report's handoff note and verification receipts to
  `work/briefs/handoff-NAME.md`; update the durable task state with unresolved
  failures, dependencies and the next action. Keep same-track follow-up
  with its author; replace the author only when unavailable, context is
  insufficient or the work has become a separate track. Point a replacement
  at that note. Use one evolving receipt and a concise final ticket update.
  Fix trivial rebase fallout yourself.
- Run at most 2 tracks that build at once, until the maintainer says otherwise: tracks spend most of their time waiting on builds. A read-only track, one that runs no build, is exempt. Run at most 5 agents at once in all.
- A review of a large surface always finds something, so "until a review is clean" does not end. A blocker is a defect this track's diff introduces or makes reachable, that loses acknowledged data or gives a wrong answer through normal use. Anything else, including a defect the base already has, goes on a ticket, and severe ones go first in the queue. Holding a track back does not protect anyone from a bug already on main. A track gets at most one delta review after its redesign, limited to that round's diff; a further blocker is fixed and checked with no new review. Past that, land or abandon it, and say which.
- After a review with blockers, review the delta yourself or with a fresh
  agent; a blocker-free round needs no new full review. Run the full check
  only on the final round.
- A delta round that adds a mechanism the review never saw (a latch taken while another is held, a retry loop, a new thread or wait) gets a scoped review of that mechanism before landing, even after a blocker-free review. A fix round that held two page latches at once hung `find` against prune.
- Overlap independent review of a stable snapshot with targeted tests and
  mutations of the candidate. Give the reviewer an immutable revision; keep
  mutation edits out of its view. Run the final full check after both converge.
  When the author continues rewriting a jj change, duplicate the frozen revision
  onto its existing stable parents with `jj duplicate REV`, then check out an
  empty child of that duplicate in the review workspace. Compare its full tree
  with REV and record the empty diff. An ordinary child of the author's change
  rebases when the author amends it; a hash in the brief does not pin that checkout.
  Keep the original revision as evidence provenance and never land the review copy.
- Give every track that changes code a review by a fresh agent, in a child workspace, before it lands: the author's own checks do not count. Tell it to form its view before reading the receipt. Give a review that builds a slot, moved to the commit with `jj new REV`, so it reuses a build directory.
- A review that runs no build is a read-only track, and its verdict rests on reading. Check yourself anything it says it could not confirm, such as the provenance of expected values, and rely on the author's mutants and the full check for the rest. Prefer a review with targeted builds and mutants when build capacity allows, and for `unsafe` or concurrency code.
- A small edit of your own after a blocker-free review, such as a test or a stated precondition, needs no second review. Run a new test against the mutant it is meant to kill, and pass it through the full check.
- Commit an edit in the main checkout before the next command: a jj operation in a slot that runs beside one in the main checkout leaves the main working copy stale. To recover, copy the edited files aside, run `jj workspace update-stale`, and compare them before going on.
- When a review finds an older wrong answer next to the change, a third in one subsystem means the cause is a class there, not a site: brief an audit of that subsystem for the class before the next fix in it.
- A follow-up a review files that changes no answer, loses no data and opens no hang goes on the track's follow-ups ticket at its priority, not above the queue's wrong answers. Close a follow-ups ticket as wontfix when nothing on it has moved for a month of tracks.
- Land a performance track on its measured saving, against a stop line in its brief. A change that adds code and does not move its measure stays unlanded: record why on the ticket, keep the commit, and free the workspace. Compare CPU time across sessions only with the load noted: load alone has moved it by about 20%.

## Land

Review architecture as well as behavior. Before accepting custom orchestration,
compare it with the underlying tool's native configuration and extension points.
Each wrapper, watcher and staging layer needs a concrete unmet requirement.
Ask a fresh reviewer to challenge those layers and their responsibility placement.
Passing tests do not justify unnecessary machinery.

1. Review the workspace's diff against its brief and the rulings, and run
   `work/tools/house-check DIR --from BASE --to NAME@`, with DIR the track's directory, and the track's base.
   Without a range it checks only the working copy, which is empty once the agent commits.
2. Run `work/tools/land NAME 'SUBJECT'` with the workspace name, not its
   directory. Propose SUBJECT to the user first unless they allowed
   commits without it. It snapshots the workspace, refuses while the
   main working copy has changes, rebases the track onto the main
   checkout's parent and moves `main`. A slot becomes free for the next
   track, keeping its directory and target/; any other workspace is forgotten.
   Agents that file tickets write into the main working copy
   at any time, so when `land` refuses, describe those changes and run
   `jj new`, then run it again.
3. When `land` reports conflicts, send the track back to its agent with
   the commits that landed since its base.
4. Verify the landed tree against the final checked revision with
   `jj diff --from TESTED --to main --summary`. An empty diff reuses that
   passing receipt; record the comparison and do not rerun `check`. If landing
   changed inputs, review the delta and rerun affected checks. Run a new full
   check only when the delta's impact cannot be bounded; state why. Apply the
   [receipt reuse rules](verification.md#verification-receipt) to prose-only
   deltas. Fix trivial failures yourself; ticket the rest for a new track.
   Close the ticket with a dated section. Leave a slot's directory in place.
   Remove a workspace's directory outside the slots once nothing in it is needed.
5. Once landing verification passes, for a workspace outside the slots, delete its
   `target/debug` and `target/tmp`, each by its literal absolute path in
   its own command. Keep the other files and directories under `target/`. A
   slot keeps them: `slot` empties a reused slot's `target/debug` and
   `target/tmp` when together above `ADTB_SLOT_CAP_GB`, default 20.
6. Read what is about to be published before every push, tickets and
   `work/` included: the repository is public, and a push cannot be taken
   back. Run `jj diff -r 'main@origin..main'` and take out credentials,
   personal data and machine details. Move them to `work/briefs/`, or
   leave them out.
   Read before the push, as its own command, never in the same one.
7. Push `main` to GitHub and to the forge: `jj git push --remote origin
   --bookmark main`, then `jj git push --remote forge --bookmark main`.
   `origin` is git@github.com:tonyd33/adtb.git; `tangled` is the old
   remote and is no longer pushed.

Any other deletion needs the user's approval. When `workspace-gc --guard`
fails, first run `work/tools/workspace-gc --clear-build main`: it empties the main
checkout's `target/debug` and `target/tmp` once idle for `ADTB_GC_IDLE_HOURS`, refusing while
cargo or any process uses it, and costs one rebuild. `ADTB_SLOT_CAP_GB=0
work/tools/workspace-gc --clean` also takes idle free slots. Otherwise list the
candidates without deleting: the
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

## Follow-up queue

Follow-up tickets must not outgrow the work that clears them. Report the open count, split by bug and feature, when a milestone status is asked for.

- With two tracks, one works the queue, ahead of milestone work: a batch of tickets sharing a cause for one agent, lowest priority number first, with the mechanism of [its class](defect-classes.md) before its instances. With one track, alternate.
- A review finding in the code of the track goes in that track's next round. File a ticket only for what lies outside the track, and only when it names an observable defect or a concrete measured cost. Note style findings and ideas in the receipt, and do not file them.
- A batch that closes tickets also closes what it finds obsolete or duplicate, by reading the code, with a dated line saying why.

## Context

Keep the current handoff's verified facts, assumptions, active blockers,
owners and next actions distinct. Archive superseded narrative so old
workarounds cannot masquerade as current instructions.

Checkpoint at meaningful task boundaries and before the active session
approaches its context limit or compaction threshold. Use the capacity
and warnings the harness exposes; do not assume a fixed token budget.
Follow [the evidence workflow](verification.md) for receipts and durable
state. Keep `work/HANDOFF.md` short and current, archiving superseded
narrative without losing unfinished work. On resume, verify the live state;
historical permissions in a handoff are not authorization for a new session.
