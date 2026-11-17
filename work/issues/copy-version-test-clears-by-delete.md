---
created-at: 2026-11-16
status: open
kind: bug
priority: 600
summary: "the CopyVersion case of every_heap_write_clears_the_all_visible_flag deletes the version first, so the delete clears the flag and the copy is not shown to"
tags:
  - test
  - class-weak-tests
---

# The CopyVersion case clears the flag by its delete

Found by the audit-candidates triage, 2026-11-16, by reading. The `CopyVersion` case of `every_heap_write_clears_the_all_visible_flag` deletes the version before copying it, and the delete already clears the source page's all-visible flag, so the case passes whether or not the copy clears it. The code does clear it (heap/write.rs `HeapWrite::commit`).

Fix direction: set the flag after the delete and before the copy, or copy a live version, so the copy alone is what the case observes.

Defect class: Tests asserting less than they claim.
