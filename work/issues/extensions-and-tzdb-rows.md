---
created-at: 2026-10-27
status: open
kind: feature
priority: 878
summary: "install and adopt tzdb write no extensions, wasm_modules, tz_releases or zones rows, and adopting a release changes no zone rules"
depends-on:
  - install-package-bytes
tags:
  - engine
---

# Feature: catalog rows for packages and time zone releases

## Report

`install` and `adopt tzdb` resolve and hash their file with BLAKE3, but the engine never fills `Source::packages`, so both are refused. Once it does, write the `extensions`, `wasm_modules`, `tz_releases` and `zones` rows, and make an adopted release's zone rules the ones conversions use.
