---
created-at: 2026-10-27
status: open
kind: feature
priority: 874
summary: "interior directory holes and an emptied last level-0 directory page stay until an insert fills them"
tags:
  - engine
---

# Feature: free interior heap directory holes

## Report

`heap/dir.rs` `trim` drops vacated positions at the end of a level-0 directory page. Holes inside a page, and an emptied last level-0 page, stay until an insert fills them. Freeing directory pages safely needs page kind and owner checks in the unlatched readers.
