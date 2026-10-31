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
| Tests asserting less than they claim | about 20 | Diff-scoped mutation in the branch check | vacuity-gate |

Language and kernel fixes mostly removed their cause, and perf and refactor tracks mostly land without blockers. The classes above concentrate in failure paths, authority and grant carry.

Open design question: grant identity by target text produced 13 rulings. The alternative is a grant as a set over catalog ids, with carry as the operator's id map and text only printed. Decide it, after checking when ids exist during a migration, before more carry work.
