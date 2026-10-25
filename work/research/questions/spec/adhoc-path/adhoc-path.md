---
title: "How data fixes and loads run"
pick: one
options:
  adhoc-write:
    title: "Ad hoc text may write for principals holding adhoc plus the write privileges"
  bundle-per-fix:
    title: "Every write goes through a registered bundle or a migration"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/adhoc-path/adhoc-write

Ad hoc text may write for principals holding adhoc plus the write privileges. Loads and staging repairs need a path; constraints hold and every change set is audited.

## spec/adhoc-path/bundle-per-fix

Every write goes through a registered bundle or a migration. A bundle per repair makes loads impractical.
