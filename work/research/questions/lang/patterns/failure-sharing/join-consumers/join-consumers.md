---
title: Every consumer of kernel terms gains a form
pick: one
raised-by: [lang/patterns/failure-sharing/join]
options:
  live:
    title: Add the form to every consumer
---
The reference interpreter, normaliser, totality, volatile and permission
analyses, translation, planner, executor, write delta, polarity and
explain each need a case for `Join` and `Jump`, and the normaliser must
keep jumps in tail position through its rewrites (for-of-if pushes the
`for` into both branches, and so into the join body).

## lang/patterns/failure-sharing/join-consumers/live

Pay it. The form also serves any later lowering that duplicates a
continuation.
