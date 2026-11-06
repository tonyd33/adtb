---
created-at: 2026-11-05
status: open
kind: bug
priority: 640
summary: "security.md says a registered call's audit args hash the wire encoding at the bundle's version, but binding.rs hashes the ad hoc text encoding"
---

# Registered call audit args use the ad hoc encoding

## Report

## 2026-11-05

From the S5.6 review, predates it. `binding.rs:350` computes `args` with the same `args_hash` as ad hoc text (one name=value line per parameter, canonical text, sorted), while security.md's Call row paragraph says a registered call hashes the arguments in the wire encoding at the bundle's pinned version. Decide which, and make code and spec agree.
