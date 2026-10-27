---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 173
summary: "merge does not carry checks, install adds an empty module, stored-view admission is unenforced, feeds skip the volatility rule"
closed: 2026-10-27
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

## 2026-10-26: done

Landed in `crates/adtb-lang`:
- a merge carries a check over some but not all merged constructors
  into a check on the merged constructor matching on the
  discriminator, `true` for the others, with its fields renamed as the
  routing carries them; a check reading a field the routing computes,
  or over merged and unmerged constructors, is refused;
- `install` reads its package from `Source::packages` (the text of a
  module file without its `module` line) and adds its declarations;
  a missing package is refused. Revert of `install` uninstalls, revert
  of `uninstall` restores the declarations;
- log grants are stored in `Source::grants` by catalog id, a `revoke`
  removes them, and a grant's field list must name fields;
- `check::check_feed` refuses a feed naming a volatile relation; the
  spec corpus checks every feed;
- `Refusal::recheck_from` locates a recheck error in text an operator
  copied from the migration; the corpus reports it at its line.

Still open:
- stored-view admission: the engine has no stored views, `store` has
  no resolved form (`resolved-operators`), and admission needs the
  engine's bounded planner. The test
  `storing_a_view_with_no_index_bounded_maintenance_is_refused_naming_the_index`
  in `adtb-sim/tests/constraints.rs` fails until then;
- the `.adtx` package format is unspecified, so nothing fills
  `Source::packages` from a file yet;
- structural operators do not carry log grants (a merge refused until
  it restates grants, drops of granted objects);
- a recheck error in text an operator did not copy from the migration
  is still reported at the block.

## 2026-10-27: resolved

## 2026-10-26: landed

Landed with the evof track: merge carries checks, install reads its package, log grants are stored in the source, feeds follow the volatility rule, and recheck errors are located in copied text. Stored-view admission moved to `stored-views`; other follow-ups are in `install-package-bytes`, `migration-checker-follow-ups` and `spec-evolve-gaps`.
