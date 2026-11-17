---
created-at: 2026-11-10
status: resolved
closed: 2026-11-17
kind: bug
priority: 450
summary: "Version::refuse's judge slices a version header without a length check, so a normal slot shorter than a header panics under the page's exclusive latch"
tags:
  - quality
  - engine
  - class-failure-flattened
---

# Bug: Version::refuse can panic on a short slot under its latch

## Report

The judge in `Version::refuse` (crates/adtb/src/heap/handle/mod.rs:52-58) calls `Header::read(&page[at])`, which slices `version[..HEADER_LEN]` (heap/version.rs:25). A `Slot::Normal` shorter than `HEADER_LEN` on a page that passes the frame check panics while the exclusive latch is held. `names_writer` in the same file (:107) guards with `bytes.len() >= HEADER_LEN` and filters by owner; this judge takes any heap-kind page through `HeapPage::any` without checking the owner.

Fix direction: guard the length and the owner as `names_writer` does, so a short or foreign slot is judged out of frame, not a panic.

Found in the quality review of `reviewed..main` at 7bf40103.

## Resolution 2026-11-17

Fixed by `fix: report slot and chunk framing that does not decode as damage of its page`, part 2 of page-decode-returns-result: `slotted::framed`, behind every `HeapPage`, requires each slot to hold at least a header, so `Header::read` never meets a short slot; a page that fails is refused as `Frame`.
