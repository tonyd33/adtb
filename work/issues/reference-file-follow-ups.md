---
created-at: 2026-10-26
status: open
kind: feature
priority: 286
summary: "the reference has no Parquet or Arrow scans, no file pinning, no constructor translation by file version and no newtype domain checks on read"
tags:
  - reference
---

# Feature: Reference file scan follow ups

## Report

Left by the reference file scans track:
- Parquet and Arrow scans return `NotYet`; they need a reader crate.
- File pinning across statements (`FileChanged`) does not exist.
- Constructor names are not translated by the file's version.
- Newtype domains are not checked when a file is read.
- The differential tests could give the reference an engine-derived row measure through `with_row_limit`.
