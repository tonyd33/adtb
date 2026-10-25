---
title: "What the schema log holds"
pick: one
options:
  authored:
    title: "Only author-written migrations; retirement, leases and supersession are deployment state"
  engine-appends:
    title: "The engine appends retirements, rebinds and reverts to the log"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/log-content/authored

Only author-written migrations; retirement, leases and supersession are deployment state. Engine appends diverge the file from shipped migrations and fail the next open.

## spec/log-content/engine-appends

The engine appends retirements, rebinds and reverts to the log. The next open fails Diverged and fleets refuse tenants.
