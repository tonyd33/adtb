---
created-at: 2026-11-16
status: resolved
closed: 2026-11-16
kind: bug
priority: 160
summary: "a registered query or transaction is checked without the program's function rows, so field access and key over a function's rows are refused there and accepted ad hoc"
tags:
  - lang
  - class-copied-rules
---

# A registered query or transaction is checked without the function rows

Reproduced 2026-11-16 by the audit-candidates triage (receipt-audit-candidates-r1, local) on main 2efdbea4. `check_query_in` and `check_txn_in` build their `Checker` without copying `fn_rows`, which `checked_items` sets. The same body is accepted ad hoc and refused registered: field access through a function's rows ("Customer has no field `loud`") and `key n` over them ("`key` needs a row whose relation is known here").

`checked_items` also sets `program`, `bundles`, `late_writer` and `hidden`, which the registered paths do not.

Fix direction: one `Checker` constructor from the program used by `check_query_in`, `check_txn_in` and `checked_items`, deciding each of those fields once; a test that checks the same bodies ad hoc and registered and asserts the same outcome.

Defect class: Wildcards and copied rules.

## 2026-11-16, resolved

Landed as lzvwznnq 2a20085f (2 rounds), track checker-one-gate: registered bodies are checked through `Checker::of_program` (with `Checker::with_rows` for schema bodies), so no path sets checker context by hand; a search checks the same read and write bodies ad hoc, as a registered query and as a registered transaction, with one outcome each. Main's full check: 3859 of 3860, one timeout under two-track load (laws-revert-test-near-timeout), which passes alone in 29.5 s.
