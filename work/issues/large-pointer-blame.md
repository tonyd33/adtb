---
created-at: 2026-11-05
status: open
kind: bug
priority: 520
summary: "large.rs read, chunks and update damage the page a large-value pointer names, though the pointer may be the liar"
---

# Large-value reads blame the page a pointer names

## Report

## Report

The directory blame rule (damage the named page only if its header claims the position; otherwise the page holding the pointer) now governs every heap path. `crates/adtb/src/heap/large.rs` `read`, `chunks` and `update` still damage a page a large-value pointer names, so a corrupt pointer in a heap row naming another holder's page (a map page, another heap's page) damages that innocent page and quarantines its holder. Same class as the heap-directory fix (identity trusted from an unvalidated pointer). Noted by the read-blame author, 2026-11-05; not reproduced.

## Proposal

Check the named chunk's header (large-value kind, the owning store, the expected chunk position) before blaming it; otherwise blame the page holding the pointer. Extend the search test in heap/dir.rs to large.rs's damage calls.
