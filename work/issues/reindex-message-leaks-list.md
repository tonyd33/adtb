---
created-at: 2026-10-29
status: resolved
kind: bug
priority: 800
summary: The reindex refusal prints an internal empty index list
closed: 2026-10-31
---

# The reindex refusal prints an internal empty index list

## Report

Found by the manual-gaps track on 2026-10-29. `reindex` on `pets` answers "rebuilding indexes []
of `pets` does not run in this engine yet". The `[]` is an internal list. Fix: name the relation
only, or the indexes by name.

## 2026-10-31: closed by triage

Obsolete. api/command.rs:315 calls reindex_job and the message is gone.

## 2026-10-31: resolved
