---
created-at: 2026-11-07
status: open
kind: bug
priority: 300
summary: "PageExclusive::inner is pub(crate) and hands out pages::Exclusive, whose write is pub, so any adtb module can change a page unlogged; the logged-change invariant holds by convention only"
tags:
  - class-not-atomic
---

# Raw page guards leak out of the pager

## Report

## 2026-11-07

A resident page's bytes change only through a logged change (`Pager::apply`: log the edits, set the LSN, keep the checksum), plus three writers with their own rules: recovery replay, the blank fill at allocation, and the damage paths that zero or restore a copy. Nothing enforces it.

- `PageExclusive::inner` (crates/adtb/src/storage/pager.rs) is `pub(crate)` and returns `&mut pages::Exclusive`, whose `write(offset, bytes)` is `pub` in adtb-sys. Any module of adtb holding a `PageExclusive` can write the page unlogged.
- `PageExclusive::g` is `pub(super)`, so the rest of `storage` reaches the raw guard directly.
- Every caller today is inside `storage/` and passes the guards to a logging primitive (the batch apply takes `&mut [&mut pages::Exclusive]`, which is why callers unwrap). No live bug found; checked by grep.

Direction: the batch entry points take `&mut [PageExclusive]` and unwrap inside the pager; `inner` and `g` become private to pager.rs, with a constructor for the blank frame alloc.rs builds; the unlogged writers are named pager methods; `disallowed_methods` in clippy.toml refuses `adtb_sys::pages::Exclusive::write` outside the files that hold them.

Defect class: Multi-step change not atomic under failure, crash or race.
