---
created-at: 2026-10-27
status: resolved
kind: feature
priority: 843
summary: the campaign app runs its bundle bodies as ad hoc text instead of calling generated code
closed: 2026-10-27
tags:
  - app
---

# Feature: port the campaign app to generated calls

## Report

`adtb-codegen` generates and compiles about 4400 lines for `apps/campaign`. Its tests still run bundle bodies as ad hoc text, because `campaign-1.0.adb` pins version 1 and calls of an earlier-pinned bundle are refused `Untranslatable` until `version-translation` lands. Write a bundle pinned at the app's current version with its hash, add a `build.rs` with `adtb-codegen` as a build dependency, and rewrite the 54 `app.call`, `ok` and `one` sites in `tests/engine.rs` to typed calls.

## 2026-10-27: done

`bundles/campaign-1.3.adb` pins version 4 with its hash and follows the 1.2 split and the 1.3 quest states: `add_region`, `add_settlement` and `add_site` replace `add_location`, the quest queries gain `Aside` and `Dropped`, and it adds `shelve_quest`, `resume_quest`, `abandon_quest`, items, maps and pins. `campaign-1.0.adb` stays as history. `apps/campaign/build.rs` generates modules `schema`, `v1_0` and `v1_3` into the library. Every test in `tests/engine.rs` calls through generated code; the tests that stop at releases 1.1 and 1.2 use ad hoc text, since no bundle pins versions 2 or 3. `release_1_3_calls_through_its_bundle_over_a_release_1_0_file` opens at 1.0, reopens at 1.3 with its bundle, checks that the 1.0 bundle is superseded, and calls through `v1_3`. No change to `adtb-codegen` or `adtb::typed` was needed.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the port track.
