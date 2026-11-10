---
created-at: 2026-11-10
status: open
kind: bug
priority: 480
summary: "an open whose marks page does not decode marks the database in memory but neither refuses nor records the page, and every later mark fails naming Checksum"
tags:
  - quality
  - engine
  - class-failure-flattened
---

# Bug: a marks page out of shape is never recorded and is reported as Checksum

## Report

`Pager::load_marks` (crates/adtb/src/storage/quarantine.rs:470-483) reads the marks page under a shared latch and, when `decode` fails, takes `Damage::Shape(Shape::Frame)` for it, maps that through `policy::mark` to the database and loads it into `Marks` with `unreadable` set. It never refuses the page under its exclusive latch and never records the damage. `Pager::load_damage_list` (crates/adtb/src/storage/damage_log.rs, the `misfit` / `refuse_if` loop) does both for the damage list page in the same open, as storage.md §8 asks of a page "a caller holding its exclusive latch refused as out of shape".

Consequences:

- No damage record names the marks page, so the damage list, `verify` and scrub carry no `Shape` finding for it; only the in-memory Database mark shows anything.
- `Pager::change_marks` (quarantine.rs:715-729) answers `Unwritable::Unreadable` with `self.read(page).err().unwrap_or(PageError { damage: Damage::Checksum })`. The page reads fine, so every later `quarantine` or `release` fails as `MarkFailure::Page` with `Damage::Checksum`, a damage the page does not have. `Pager::replace_contents` (quarantine.rs:606-611) builds the same `Checksum` unconditionally.

Fix direction: judge the marks page as `load_damage_list` judges its page, through `refuse_if` with a `Shape::Frame` judge, so the refusal is recorded and later reads return it; then `change_marks` and `replace_contents` return the page's refusal instead of a constructed `Checksum`. A test plants a marks page of another kind or with an overlong count and asserts the recorded damage and the refusal a later mark returns.

Found in the quality review of `reviewed..main` at 7bf40103.
