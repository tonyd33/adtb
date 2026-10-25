---
title: "When a bundle lease renews"
pick: one
options:
  calls:
    title: "On binding and on calls, written at most once per lease/8"
  binding:
    title: "Only at binding"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/lease-renewal/calls

On binding and on calls, written at most once per lease/8. A long-lived connection must not count as dead while serving calls.

## spec/lease-renewal/binding

Only at binding. Pooled connections expire while serving.
