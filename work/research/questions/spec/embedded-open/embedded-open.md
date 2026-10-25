---
title: "Order of steps when an embedded program opens its file"
pick: one
options:
  register-first:
    title: "Register, supersede earlier bundles, bind host functions, migrate, plan"
  migrate-first:
    title: "Migrate, then register"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/embedded-open/register-first

Register, supersede earlier bundles, bind host functions, migrate, plan. Old bundles otherwise block shipped enables for a lease and downgrades break re-upgrades.

## spec/embedded-open/migrate-first

Migrate, then register. Enable refused at open.
