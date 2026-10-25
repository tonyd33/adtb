---
title: Stopping a session or starting vacuum is not a change to data
pick: any
raised-by: [ops/admin/actions/system-writes]
options:
  requests:
    title: The write inserts a request row; an engine worker acts on it after commit
  live:
    title: Live with it
    excludes: [ops/admin/actions/effects/requests]
---
A change set denotes inserted and deleted relations, checked by
constraints (settled `write/primitive/delta`). Deleting a row of
`sys::sessions` does not delete a session: the session is a process
state, and an aborted transaction must not have killed it.

## ops/admin/actions/effects/requests

Intent as data: `insert sys::cancel_requests { session: 4471 }`. The
row commits like any write; the worker acts and records the outcome.
Requests are auditable and replicate with the log.

## ops/admin/actions/effects/live

System writes have side effects at commit that a change set does not
describe.
