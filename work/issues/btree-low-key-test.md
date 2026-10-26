---
created-at: 2026-10-25
status: open
kind: feature
priority: 336
summary: only one seed of one test catches a missing low-key check in claim
---

# Feature: a direct test for the low-key check in claim

## Report

With `holds_from` accepting a page without `above_low`, only
`leaves_emptied` at switch rate 0.01 fails, and not at 0.05 or 0.002.
Write a test that drives a claim onto a page whose low key is above
the key, through a split or unlink between the parent's read and the
claim, so the check is caught on every seed.
