---
created-at: 2026-10-28
status: wontfix
closed: 2026-11-18
kind: bug
priority: 853
summary: "every campaign engine test takes 1 to 11 s, most over 4 s, against the 2 s default for a test"
tags:
  - test
---

# Campaign engine tests are slow

## Report

The campaign-tests track measured them inside a full check: apps/campaign/tests/engine.rs tests run 1.0 to 11 s each, most of them 4 s or more, at base cd020ae0 as well. Find where the time goes (opening and migrating the database per test, the 13 s four-migration open recorded in migration-open-slow, or fixture setup) before changing anything.

## 2026-11-18: wontfix

The campaign app is removed by maintainer ruling (2026-11-17); its tests go with it. The upgrade-across-releases coverage it carried is filed as release-upgrade-coverage.
