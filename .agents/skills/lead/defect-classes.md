# Defect classes

Causes that keep producing bugs across tracks, from the review of 2026-10-31. Counts are hand counts over fix commits and bug tickets, about 20% either way. Add a class when a third unrelated instance appears. Remove one when its mechanism has landed and a test or the compiler catches a new instance.

| Class | Instances | Mechanism | Ticket |
|---|---|---|---|
| Multi-step change not atomic under failure, crash or race | about 20 | Build aside, publish last, memory updated under the latch | fault-matrix |
| Damage or failure gives a panic, hang or flattened error | about 24 | Typed fatal payload and causes, read checks return `Result` | fault-matrix |
| Authority and admission checked per entry path | about 18 | One admitted value required by every execution entry | admitted-gate |
| Unbounded work over size, depth or sharing | about 16 | One budget, sized stacks, sharing-aware lowering | one-spend-budget |
| Wildcards and copied rules | about 17 | Scoped lint, shared helper | compile-time-identity |
| Catalog rows and identity by position or spelling | about 16 | Row codecs generated from `catalog.adt`, keyed by id | compile-time-identity |
| Tests asserting less than they claim | about 20 before the audit, about 80 candidates after | Must-use tallies and floors, a reason helper, gates that run, diff-scoped mutation | test-audit-findings, vacuity-gate |

Language and kernel fixes mostly removed their cause, and perf and refactor tracks mostly land without blockers. The classes above concentrate in failure paths, authority and grant carry.

Open design question: grant identity by target text produced 13 rulings. The alternative is a grant as a set over catalog ids, with carry as the operator's id map and text only printed. Decide it, after checking when ids exist during a migration, before more carry work.

## Mechanism evidence through `bd61e6fc`

No class is retired by the three recent implementation fixes. The counts
above remain historical estimates, not a current inventory.

- Typed `FatalIo` removes diagnostic-text classification across current
  storage producers, the shared process helper and the host callback catch.
  Direct fault execution inside maintenance workers remains unverified;
  other flattened errors and corruption causes remain open.
- Noncreating host fallback makes creation provenance come from exclusive
  creation. It does not carry ownership through every later acquisition
  refusal; `open-acquisition-create-cleanup` remains a source-only candidate.
- Pinned upserts now retain the checked constructor selection and enforce it
  in both evaluators. This repairs a static/runtime domain disagreement;
  nonhiding admission and nested-upsert reader analysis remain open.
- Exact assertions, reached-case floors and tool-test discovery strengthen
  the checked cases. They do not establish a general vacuity gate or remove
  the catalog-identity, admission and resource-budget causes.

Measured regressions and mutants support these bounded enforcement points.
There is not yet enough subsequent work to measure class-wide recurrence.
