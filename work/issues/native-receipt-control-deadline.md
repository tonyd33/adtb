---
created-at: 2026-11-01
status: resolved
kind: bug
priority: 400
summary: native receipt can time out after a successful payload
closed: 2026-11-01
tags:
  - quality
  - tooling
---

# Native receipt control deadline

## Report

A successful native payload can lose its resource and result receipts because
ExecStopPost shares the 200 ms payload termination allowance. Raising that timeout
alone also lets a TERM-ignoring payload run beyond its declared active deadline.

Acceptance: separate native payload termination from administrative receipt time;
retain memory, swap and task containment and invalid incomplete outcomes. A real
receipt delayed beyond 200 ms must complete within the control allowance, while a
TERM-ignoring parent and descendant cannot perform a later observable effect.
Tests must reject the old receipt allowance and the naive timeout-only repair.

## 2026-11-01: resolved

Separate receipt administration from native payload termination: the manager sends
SIGKILL to the service control group at the active deadline, while the receipt
gets the existing two-second control allowance. Memory, swap and task budgets
remain unchanged. The protocol counts all five possible native stop states.

Four real native sensitivity cases cover delayed receipt success, failure with
the old 200 ms allowance, hard termination of a TERM-ignoring parent/descendant,
and late effects under the naive timeout-only mutant. Ten boundary cases, three
preexec resource guards, two collector outcomes and shell checks pass. Independent
source and method review accepted this bounded repair. Native executed inputs are
byte-identical after rebasing over the unrelated cache-retention repair.
