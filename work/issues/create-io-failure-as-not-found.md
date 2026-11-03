---
created-at: 2026-11-02
status: open
kind: bug
priority: 320
summary: Pager::create maps every open failure to NotFound, which the api reports as Corrupt
tags:
  - quality
  - engine
---

# Create I/O failure as not found

## Report

Found by the audit-floors track. `Pager::create` maps every failure opening the data file to `OpenError::NotFound`, and `failed_create` turns that into `Corrupt { detail: "NotFound" }`. An I/O failure creating the file is reported as damage. Same class as the `NoHeader` fix that track landed for `Pager::open`: map the failure to `OpenError::Io` and the api's `OpenError::Io` with its kind, and test a create under a path whose parent is a regular file.
