---
created-at: 2026-11-05
status: open
kind: feature
priority: 600
summary: registration scripts pin sim-clock instants that move with any change to the I/O done at open
---

# Feature: Scripts pin sim-clock instants instead of their relations

## Request

## Proposal

## Open questions

## 2026-11-05

crates/adtb-sim/scripts/registration/ (binding/call-renews-when-served.test, open-never-renews-unpresented.test, reregistering-needs-the-owner.test and one more) pin `catalog::bundles.renewed` instants. The clog-lost change moved them twice, by 41 to 130 ms and then under 1 ms, only because the open read the log; a reviewer had to confirm by hand that no outcome changed. The contract is relational: renewed equal across the lines that do not renew, later than registration after a renewal. Give the script runner a way to bind an instant and compare later values against it, and rewrite these scripts to use it.
