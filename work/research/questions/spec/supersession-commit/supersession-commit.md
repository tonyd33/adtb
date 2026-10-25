---
title: "When supersession at an embedded open commits"
pick: one
options:
  before-migrations:
    title: "Durably and on its own, before the shipped migrations; undone only by re-registering; registrations stay atomic"
  atomic:
    title: "Atomic with open success"
---
Raised by the third review of the spec in docs/spec/.

## spec/supersession-commit/before-migrations

Durably and on its own, before the shipped migrations; undone only by re-registering; registrations stay atomic. A failed registration never revives an old reader over an enabled constructor.

## spec/supersession-commit/atomic

Atomic with open success. Rolls back after migrations committed.
