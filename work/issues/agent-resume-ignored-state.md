---
created-at: 2026-10-27
status: open
kind: bug
priority: 65
summary: essential agent briefs and handoffs live in ignored files and the local checkpoint mixes old state with prior permissions
tags:
  - tooling
  - devex
---

# Agent resumption relies on ignored session state

## Report

`.agents/skills/lead/SKILL.md:24-25` puts briefs in ignored work/briefs; lines 40-42 put successor handoffs there too. `.gitignore:19-21` excludes work/HANDOFF.md, work/briefs and local quandary state. At audit, work/HANDOFF.md had 528 lines combining a latest checkpoint, superseded history and permissions from an earlier OpenCode session. A fresh clone cannot recover ignored briefs, and another harness must distinguish historical permissions from current authorization.

Keep essential resumable task state in tracked tickets or other tracked supporting files, consistent with docs/development.md:112-116. Retain an optional short local checkpoint linking to durable state. Clearly label historical authorization and avoid treating it as a grant to a new session. Verify that a successor with only tracked files can locate task scope, acceptance checks, unresolved failures and landing dependencies.

## 2026-10-28: workflow guidance and local checkpoint

Added `.agents/skills/lead/verification.md` with state-bound check receipts,
separate permission/queue/execution timing, explicit file-tracking verification
and current-session authorization rules. Lead/worker and development guidance
link to it. Archived the prior local handoff byte-for-byte at
`work/briefs/handoff-history-2026-10-28-before-evidence.md` and shortened
`work/HANDOFF.md`, preserving the unlanded campaign-prep state and incomplete
checks. No campaign files or build artifacts changed.

Still open: essential existing ignored task records have not been migrated;
a clone-only successor test for a real in-progress track remains required.
The new guidance is not proof that those existing records are tracked.

## 2026-10-28: concrete ruling dependency omission repaired

Both fresh workers found isolated quandary rulings refused: tracked
points/current.md refers to spec/key-too-large/rejected, but the historical
question file was ignored in main and absent from sibling workspaces. Track
that existing question byte-for-byte; no rulings changed. Successors use main's
authoritative research space until rebased. This repairs the concrete missing
reference, not the broader clone-only resumption acceptance above.
