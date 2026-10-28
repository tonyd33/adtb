# Verification and resumption evidence

## Verification receipt

Record one receipt per check scope in the track's handoff. Link retained
logs rather than copying their output. Include:

- State: workspace path, full tested commit id and base revision. Snapshot
  with `jj status` before the check and record the id with
  `jj log -r @ --no-graph -T 'commit_id ++ "\n"'`. Check the state again
  after the command; report any changes to its inputs during execution.
- Scope: exact command, working directory, relevant toolchain, profile,
  features and environment overrides. Distinguish the root workspace from
  `apps/campaign`.
- Outcome: passed, failed, timed out, interrupted, blocked or not run;
  command exit status when collected; failing tests or incomplete stages;
  retained log path. A successful early stage is not a complete check.
- Timing, when observed: total elapsed time, permission wait, queue/lock
  wait and execution time separately. Mark unavailable measurements as
  unknown. Do not attribute a whole tool-call span to execution or infer
  contention from elapsed time alone.
- Remaining work: checks still required and who owns them. The lead owns
  the post-land integration check.

For example:

```text
State: /path/to/adtb-track, tested COMMIT, base BASE; inputs unchanged
Scope: DEVENV_ROOT=$PWD CARGO_TARGET_DIR=$PWD/target check; workspace root
Environment: toolchain TOOLCHAIN, profiles/features as configured
Outcome: interrupted, exit 143; clippy passed, root tests did not run
Evidence: target/check-RUN.log
Timing: elapsed DURATION; permission/queue/execution unknown
Remaining: full check, independent review, lead post-land integration check
```

A branch receipt does not certify a changed combined tree after rebase or
conflict resolution. Rerun affected checks and retain the lead's integration
check. For a prose-only delta, reuse a receipt only when the brief permits
it and the tested inputs, including embedded examples and build inputs,
are unchanged; record the receipt and the delta reviewed.

An isolated pass does not erase a suite timeout. Report both. Permission
wait remains part of total elapsed time but is not execution time; report
it as permission wait when that cause is established. Never bypass approval
in order to reduce the measured delay.

## Durable task state

Keep the scope, acceptance commands, workspace/base, unresolved failures,
landing dependencies and next action in a tracked ticket or supporting
note before relying on it for another machine or harness. Include the
receipt or enough evidence to rerun the checks if local logs are absent.

Verify that the essential paths appear in `jj file list PATH...`; being
present on disk or readable by tix does not prove a file is tracked.
Explicitly track only the needed files with
`jj file track --include-ignored PATH...` when appropriate. Do not unignore
or import the whole local history. Do not publish private transcripts or
credentials in a durable note.

Keep `work/HANDOFF.md` as a short current checkpoint linking to durable
state and any local evidence. Archive superseded narrative under
`work/briefs/` before shortening it. Label unlanded work and incomplete
checks explicitly; preserve source edits and required artifacts.

On resume, read the current user direction and inspect live jj/ticket state.
Historical permissions in a handoff or archive do not grant authorization
to the new session. If ignored notes are absent, use durable state and
report missing evidence rather than inventing a successful check or a task
boundary.
