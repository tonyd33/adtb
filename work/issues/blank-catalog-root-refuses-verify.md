---
created-at: 2026-11-02
status: open
kind: bug
priority: 410
summary: A blank catalog root fails open with Corrupt where the spec opens it for verify
---

# Blank catalog root refuses verify

## Report

Found by the page rot search, see fault-matrix. A catalog root page reading as blank fails `Database::open` with `Corrupt`. The spec, 11.2 per the author's receipt, says such a file opens for `verify`. The refusal is typed and loses nothing, so it is a gap in repair, not a wrong answer. Decide which side moves.
