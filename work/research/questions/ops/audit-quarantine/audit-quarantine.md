---
title: "What a call does when a constructor of the audit relation is quarantined"
pick: one
raised-by: [ops/halt-scope/heap]
options:
  reads-answer:
    title: "A call that writes data fails `Quarantined`; a read, a refusal and `verify` answer, with their rows unwritten"
  every-call-fails:
    title: "Every call that would write an audit row fails"
  audit-skipped:
    title: "Every call runs and no row is written, as under a whole quarantine"
---
A review found that a mark on `catalog::audit.Call` fails every task,
`verify` included, against catalog §11.2's "every other call continues".

## ops/audit-quarantine/reads-answer

A committed change with no audit row cannot be told from tampering, so a
write that cannot record itself must not commit. A read changes nothing,
and its failure would stop the operator from reaching `verify` and the
rest of the data over one damaged page, while the mark itself shows that
rows are missing.

## ops/audit-quarantine/every-call-fails

One page of the audit relation becomes a whole-database outage, with
`verify` unreachable over the server.

## ops/audit-quarantine/audit-skipped

Changes commit unrecorded for as long as the mark holds.
