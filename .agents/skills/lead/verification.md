# Verification and resumption evidence

## Verification receipt

Record one receipt per check in the track's receipt file:

- State: workspace, tested commit id and base.
- Command: the exact command and its directory, the root workspace or `apps/campaign`.
- Outcome: exit status, failing tests or unfinished stages, and the log path.
- Remaining: checks still owed and who owns them.

After landing, record `jj diff --from TESTED --to main --summary`. An empty diff reuses the
receipt. A tree changed by a rebase reruns the checks its delta reaches; a prose-only delta reruns
only the checks that read it. An isolated pass does not erase a suite timeout: report both.

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
