---
created-at: 2026-10-25
status: open
kind: feature
priority: 742
summary: "exclusions probe their equality prefix and filter on overlaps; one with no equality fields is refused though the spec admits it"
tags: [engine]
---

# Feature: interval index probes

## Report

An `overlaps` exclusion probes the interval index's equality prefix, then filters. An exclusion with no equality fields is refused by admission, which `docs/spec/constraints.md` admits. Add an interval probe to the planner and the index.
