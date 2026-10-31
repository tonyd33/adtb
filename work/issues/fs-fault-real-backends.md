---
created-at: 2026-10-29
status: wontfix
kind: feature
priority: 740
summary: no fault-injecting file system tests on the real backends
closed: 2026-10-31
---

# Feature: Fault-injecting file system tests on the real backends

## Request

Milestone 9, docs/design/runtime.md 7.2: faults under the real engine on the real backends. uring-follow-ups records that faults cannot be injected under io_uring. os-fault-fs is resolved; say what it leaves.
## Proposal

## Open questions

## 2026-10-31: closed by triage

Duplicate of real-backend-faults-engine, slice S15 holds the injector and file system modes.

## 2026-10-31: wontfix
