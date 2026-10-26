---
created-at: 2026-10-25
status: open
kind: feature
priority: 800
summary: "milestone 8 (evolution and bundles, the embedded alpha) has not exited: no operator log, write states, bundles, embedded open or code generation"
depends-on:
  - history-heaps
  - milestone-7-engine
  - operator-log
  - write-states
  - registered-calls
  - bundles
  - embedded-open
  - codegen
  - schema-operator-laws
  - io-backends
  - real-runtime
tags:
  - milestone
---

# Feature: Milestone 8, evolution and bundles, embedded alpha

## Report

Milestone 8 of `docs/plan.md`, the first release. Exit: an
application can ship, evolve and reopen an embedded database through
the in-process API alone; operator laws hold over generated databases.
Needs milestone 7. Evolution is proven on the first application: its
first schema changes are the first real migrations.

Open on 2026-10-25: `operator-log.md`, `write-states.md`,
`registered-calls.md`, `bundles.md`, `embedded-open.md`, `codegen.md`,
`schema-operator-laws.md`, and `io-backends.md` and `real-runtime.md`
for a database on a real disk.
