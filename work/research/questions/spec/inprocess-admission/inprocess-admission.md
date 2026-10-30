---
title: "Whether in-process calls pass admission"
pick: one
options:
  same:
    title: "Every in-process call and ad hoc text passes the admission a server call does"
  tasks-only:
    title: "Only in-process tasks pass admission; synchronous calls run at once"
---
Raised by the spec-rev30 review (G18).

## spec/inprocess-admission/same

An in-process call waits in the same queue and is refused by the same bounds. The
synchronous call path skipping `admit` is a code gap.

## spec/inprocess-admission/tasks-only

Synchronous calls bypass the queue and the memory budget it enforces.
