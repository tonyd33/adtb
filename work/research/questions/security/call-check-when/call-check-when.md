---
title: When a call grant is checked
pick: one
options:
  submit-and-start:
    title: At submit and on leaving the queue; a started call finishes
  every-attempt:
    title: Also at each retry of a transaction
---
A revoke commits while a retried transaction is between attempts, or after a read started.

## security/call-check-when/submit-and-start

The same as ad hoc text. A call that has started finishes, whichever attempt it is on. A revoke's effect on running calls is bounded by the statement limit. Checking inside the attempt would either read `catalog::grants` in the call's transaction, adding conflicts between revokes and traffic, or read outside it and race anyway.

## security/call-check-when/every-attempt

Narrows the window by one attempt and closes nothing.
