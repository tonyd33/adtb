---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 896
summary: "BundleExpired from Database::registered or from a registration names an empty query, so its message reads expired before ``"
closed: 2026-10-27
tags:
  - quality
  - engine
---

# BundleExpired from a bind or a registration names an empty query

## Report

`Database::registered` calls `bind(name, "")` (`crates/adtb/src/api/registered.rs:600`), and `bundle::stale` builds the reason with `query: String::new()` (`crates/adtb/src/api/bundle.rs:629`). The message formatted at `crates/adtb/src/api/error.rs:466` then reads "the lease of bundle `web-1.0` expired before ``".

Fix: make the query an `Option`, or give binds and registrations a message without it, and pin the text in a test.

## 2026-10-27: done

The `BundleExpired` message leaves out the query when there is none: "the lease of bundle `web-1.0` expired". The catalog type keeps `query: Text`, as `docs/spec/catalog.md` declares. The text of both forms is pinned in `bundles::a_bundle_whose_lease_and_grace_have_passed_is_expired`.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the qwrong track.
