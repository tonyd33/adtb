---
created-at: 2026-11-10
status: open
kind: bug
priority: 420
summary: "a server call whose commit fails and whose abort record also fails can reply with a pending stop request instead of the record's failure"
tags:
  - class-failure-flattened
---

# A failed commit's record failure can be replaced by a stop request

## Report

## 2026-11-10

In api/server/call.rs the commit arm passes a failed `e.db.commit(tx)` to `retry`, which returns the owner's stop request when one is pending, before it looks at the error. Inside `write::Database::commit` a failed abort record already wins over the commit's own failure, but the call cannot tell that error from the commit's, so a cancel, idle or drop requested while the commit fails replies `Rejected Cancelled` (or the request's error) instead of the record's `Fatal Storage` or `Fatal Quarantined`. structure-transactions step 4 gave the other after-failure sites of the call the gate's two arms and left this one, because `commit` would have to say that its abort record failed.

Direction: let `commit`'s failure say whether its abort record failed (the gate's `Result<E, E>` shape), and have the call return the record arm before `retry`; a test like call_tests' stop-while-the-record-fails case for the commit path.

Defect class: Damage or failure gives a panic, hang or flattened error.
