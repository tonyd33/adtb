---
created-at: 2026-10-27
status: open
kind: bug
priority: 896
summary: "BundleExpired from Database::registered or from a registration names an empty query, so its message reads expired before ``"
tags:
  - quality
  - engine
---

# BundleExpired from a bind or a registration names an empty query

## Report

`Database::registered` calls `bind(name, "")` (`crates/adtb/src/api/registered.rs:600`), and `bundle::stale` builds the reason with `query: String::new()` (`crates/adtb/src/api/bundle.rs:629`). The message formatted at `crates/adtb/src/api/error.rs:466` then reads "the lease of bundle `web-1.0` expired before ``".

Fix: make the query an `Option`, or give binds and registrations a message without it, and pin the text in a test.
