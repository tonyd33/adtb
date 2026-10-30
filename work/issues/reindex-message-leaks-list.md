---
created-at: 2026-10-29
status: open
kind: bug
priority: 800
summary: The reindex refusal prints an internal empty index list
---

# The reindex refusal prints an internal empty index list

## Report

Found by the manual-gaps track on 2026-10-29. `reindex` on `pets` answers "rebuilding indexes []
of `pets` does not run in this engine yet". The `[]` is an internal list. Fix: name the relation
only, or the indexes by name.
