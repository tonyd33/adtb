---
created-at: 2026-10-25
status: open
kind: feature
priority: 173
summary: "merge does not carry checks, install adds an empty module, stored-view admission is unenforced, feeds skip the volatility rule"
tags:
  - lang
---

# Feature: Migration checker follow-ups

## Report

From `feat(lang): check the engine catalog modules and every migration
operator`:
- a merge refuses a per-constructor check instead of carrying it into
  one check on the discriminator (`docs/spec/evolution.md` §6.2);
- `install` adds an empty module, because the checker cannot read the
  package, and log grants are checked but not stored in the `Source`;
- stored-view admission is not enforced: `derived.md:87`, which the
  spec says is refused, checks;
- the volatility rule (`catalog.md` §1.3) does not cover feeds;
- a recheck error in a rewritten declaration is reported at the block,
  not at its line.
