---
created-at: 2026-11-05
status: open
kind: bug
priority: 560
summary: catalog.md §11.2 lists only page findings as reasons a dump of a quarantined file refuses; a commit log that cannot prove a status page also refuses it
---

# Spec: a dump refuses when the retained log cannot prove the commit log

## Report

## 2026-11-05

Since clog-lost, `admit_dump` refuses with `Fatal { reason: Quarantined { what: "the database" } }` when the retained log from the oldest status page holding an id in progress to the end recovery kept is not whole and readable. docs/spec/catalog.md §11.2's dump paragraph names only page findings as reasons to refuse. The refusal is user-observable, so state it in §11.2 through the spec-revision workflow, in line with ruling repl/dump-quarantined/prove-then-dump.
