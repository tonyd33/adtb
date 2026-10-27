---
created-at: 2026-10-26
status: open
kind: bug
priority: 893
summary: "a broken checker guarantee in a migration's data maps reaches the user as a refusal, since Failed converts every string to Refused"
tags:
  - quality
---

# A broken guarantee in a migration data map is reported as a refusal

## Report

`crates/adtb/src/catalog/change.rs:136` and `:142` implement `From<String>` and `From<&str>` for `Failed` as `Failed::Refused`, so every `?` and `.into()` on a string in the data maps reports a refusal of the migration to its author. Most of those strings in `crates/adtb/src/catalog/moved.rs` name guarantees of the checker or of `Source::steps`, not properties of the data: `field_named` (moved.rs:97), "a fill or routing takes the row" (moved.rs:105), "field has no catalog id" (moved.rs:136, 197), "a routing gives a constructor" (moved.rs:268), "the constructor's last field holds no declared sum" (moved.rs:302), the unnest shape checks (moved.rs:377, 426, 430), "the keyed constructor is gone" and "the rekeyed relation is gone" in `key_value` and `rekey`, and "has no counterpart" in `carried` (moved.rs:291). A bug there surfaces as `SchemaRefused` with a message the author cannot act on, and no test sees it as an internal error. Beside them, `rekey` (moved.rs:485) panics through `expect` on the same kind of guarantee.

Fix: drop both `From` impls, construct `Failed::Refused` only where the data refuses (a fault or rejection of a fill or routing, a routing to no product, a key that references no row, a field with no value for existing rows in change.rs:263) and `Failed::Internal` everywhere else, including moved.rs:485.
