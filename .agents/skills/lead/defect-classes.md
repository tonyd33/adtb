# Defect classes

Causes that keep producing bugs across tracks, from the review of 2026-10-31. Counts are hand counts over fix commits and bug tickets, about 20% either way. Add a class when a third unrelated instance appears. A bug ticket carries its row's tag, or `class-none`; `work/tools/slot` reads the Tag and Mechanism ticket columns, the Tag cell holds one tag and the Mechanism ticket cell one or more ticket stems, comma-separated. A class with three instances is gated while any of its mechanism tickets is open or in progress; resolved and wontfix both release it. Remove one when its mechanism has landed and a test or the compiler catches a new instance.

| Class | Tag | Instances | Mechanism | Mechanism ticket |
|---|---|---|---|---|
| Multi-step change not atomic under failure, crash or race | `class-not-atomic` | about 20 | Build aside, publish last, memory updated under the latch | fault-matrix |
| Damage or failure gives a panic, hang or flattened error | `class-failure-flattened` | about 24 | Typed fatal payload and causes, read checks return `Result` | fault-matrix |
| Damage reported as nothing: evidence of loss falls between two checks, each excusing it as the other's case | `class-damage-unreported` | 8: a page past the log end, a foreign heap page, a heap page of the wrong frame read as empty, a restore laundering a page a backup listed damaged, a backup copying a lost write as sound, a lost commit-log write settled aborted, vacuum freeing a lost-write page so verify misreports, the first verify on whole-file damage returning Fatal. The 2026-11-05 audit found the cause of most: a page is checked against the retained log only in recovery, verify, scrub and the bounded evicted table, so every other read serves a stale page and later writes and frees build on it | Each finding keyed by what the evidence proves, checked over every bound, with a search that reaches each kind; open: one read-in gate against the retained log (damage-readin-gate) and damage marked durably where it is detected | damage-marked-at-detection, damage-readin-gate, persisted-lsn-map |
| Authority and admission checked per entry path | `class-admission-per-path` | about 18 | One admitted value required by every execution entry | admitted-gate |
| Unbounded work over size, depth or sharing | `class-unbounded-work` | about 16 | One budget, sized stacks, sharing-aware lowering | one-spend-budget |
| Wildcards and copied rules | `class-copied-rules` | about 17 | Scoped lint, shared helper | compile-time-identity |
| Catalog rows and identity by position or spelling | `class-identity-by-spelling` | about 16 | Row codecs generated from `catalog.adt`, keyed by id | compile-time-identity |
| Tests asserting less than they claim | `class-weak-tests` | about 20 before the audit, about 80 candidates after | Must-use tallies and floors, a reason helper, gates that run, diff-scoped mutation | vacuity-gate, test-audit-findings |

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


## Quality cadence through `7309b60b`

The ten code commits after `36d64bd6` have a fresh bounded source review;
no new confirmed finding was filed. No defect class is retired. Typed fatal
payloads enforce cause identity at their changed producers/catchers; checked
upsert domains reach both evaluators; surviving field owners are mapped by
catalog identity. These boundaries still leave acquisition cleanup, worker
execution, nonhiding admission and nested reader facts open.

Runtime fixture migrations preserve typed outcomes, raw IDs, List shape,
external bindings and registered calls while removing ordinary case data
from Rust inputs. Discovery floors and sensitivity probes support these
specific drivers. They do not establish a universal vacuity gate, clean-build
saving or defect recurrence trend. No class-wide budget mechanism landed;
the rejected unlanded field-reader prototypes are not evidence of a fix.

## Quality cadence through `8ef7e285391b`

Ten further code commits have a fresh bounded source review, with no new
confirmed finding and no class retired. Both unique-name producers now call
one Schema provider; externally specified names cover their public boundary.
The generated-row witness checks intended payload, fresh keys and exact native
membership before comparing evaluator observations. Runtime driver discovery,
normalization and callback reachability enforce the migrated cohorts' specific
contracts. None forbids every future copied producer or weak assertion.

The comparison counter oracle checks returned values, actor order and compatible
serialization, beyond final counts. Its landed ledger remains an in-process
observation; externally acknowledged crash validation is a separate active
checkpoint outside this reviewed range. Admission, identity, shared-budget and
engine-wide failure mechanisms remain open. Fixture data no longer requires
Rust compilation on edit; this is not clean-build or whole-suite speed evidence.
