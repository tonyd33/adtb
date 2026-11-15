---
created-at: 2026-11-15
status: open
kind: bug
priority: 450
summary: "a catalog codec checks only the fields its reader reads, so a settings row's id may hold any value, and decode_val drops a value's surplus fields"
tags:
  - quality
  - engine
  - class-damage-unreported
---

# A catalog settings row with a malformed id decodes

## Report

Found by the quality review of reviewed..582d7af5 (10a, dc62bc7b). `Codec::decode` (crates/adtb/src/catalog/codec.rs:174) checks the constructor position and the field count, then hands the row to the constructor's reader; a field's type is checked only when the reader reads it (`Row::int`, `text` and the rest). No `SettingRow` reader (codec.rs:396 to 461) reads the header field `id`, so a `catalog::settings` row whose `id` holds `Text`, or an integer other than its constructor's catalog id (which `set_setting` writes, crates/adtb/src/api/command.rs:808), decodes as a valid setting. `RoleRow` reads `created` only to check its type (codec.rs:328), so the module already treats an unread field as something to check, by hand, per reader.

`Codec::decode_val` (codec.rs:197) zips the declared layout with the value's fields, so a `Val::Ctor` with more fields than its constructor declares is truncated to the declared count and then passes `decode`'s count check. Latent today: its one caller, `set_setting`, builds the row itself.

The step 10 contract is that a row whose constructor or field shape does not match the declaration is refused as damage with its whole reason.

Fix direction: have `Codec::new` record each field's stored type and `decode` check every field against it before the reader runs, so readers only convert; check a settings row's `id` against its constructor's catalog id; refuse a `Val::Ctor` whose field count is not the layout's in `decode_val`. Tests: a settings row with a `Text` id, a settings row with another setting's id, a value with a surplus field, each refused with its reason. 10b's codecs can take the same check.
