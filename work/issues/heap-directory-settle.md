---
created-at: 2026-10-25
status: open
kind: feature
priority: 558
summary: only repair corrects a heap directory left promising too little free space after a crash
---

# Feature: settle heap directories outside repair

## Report

A crash after a raise and before the entries above it are corrected
leaves an inner directory entry promising less than its child holds.
Free space under it is missed until the next raise on that path.
`verify` reports it as `Unsettled` and `repair` calls
`Heap::settle_directory`, but nothing else does. Vacuum should settle
each heap and large-value store directory it visits.
