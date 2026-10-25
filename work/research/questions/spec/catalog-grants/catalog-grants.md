---
title: "Where grants on catalog:: and sys:: relations live"
pick: one
options:
  deployment:
    title: "Deployment state, made by ad hoc statements"
  log:
    title: "Log operators like other relation grants"
---
Raised by the third review of the spec in docs/spec/.

## spec/catalog-grants/deployment

Deployment state, made by ad hoc statements. Catalog relations evolve by the catalog log, which the schema hash does not cover.

## spec/catalog-grants/log

Log operators like other relation grants. User-log grants would depend on catalog versions.
