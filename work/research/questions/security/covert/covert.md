---
title: Timing and serialization aborts carry information across policies
pick: any
raised-by: [security/policy/predicate]
options:
  out-of-model:
    title: The spec's guarantees cover results and errors; timing and abort channels are named as uncovered
  scoped-locks:
    title: Predicate locks of a policy-filtered read cover only the policy's key range
    needs: [security/leak/aligned-keys]
  live:
    title: Live with it unstated
    excludes: [security/covert/out-of-model, security/covert/scoped-locks]
links:
  - {rel: prior-art, to: prior/security-abort-channels}
---
Under `engine/ssi` a scan reads and SIREAD-locks rows its policy then
drops; a write by another tenant to those rows aborts the scanner. One
bit per attempt, which timing mitigation does not cover (Sheff et al.).
Execution time also varies with filtered row counts.

## security/covert/out-of-model

Hails claims soundness against termination and timing channels only
inside its language runtime; databases state no such property.

## security/covert/scoped-locks

The policy predicate is part of the index range (`tenant` prefix), so
range and next-key locks (`mvcc/next-key`) never touch other tenants'
entries and their writes cannot abort this reader.

## security/covert/live

Nothing is said.
