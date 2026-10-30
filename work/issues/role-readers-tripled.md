---
created-at: 2026-10-29
status: open
kind: bug
priority: 897
summary: "three new functions read catalog::roles and two map a role id to its name, each with its own decoding and error"
tags:
  - quality
  - engine
---

# catalog::roles is read three ways

## Report

The range `reviewed..df569cf7` added three readers of `catalog::roles`, one per track:

- `crates/adtb/src/catalog/grants.rs:173` `Catalog::role_names`, names in a snapshot, looking up the `name` position per row;
- `crates/adtb/src/api/bundle.rs:573` `bundle::roles`, id to name from the latest commit;
- `crates/adtb/src/api/server/tables.rs:167` `Tables::role_names`, id to name in a transaction.

Each decodes the row and reports a malformed one with its own error. Two places then map a role id to a name and fail with their own `Internal` text: `crates/adtb/src/api/server/register.rs:88` `role_name` and `crates/adtb/src/api/registered.rs:357` `definer`.

Fix: one catalog function returning the roles a snapshot sees as id to name, used by the open path and by `Catalog::role_names`. `Tables::role_names` stays over the transaction, and one id-to-name lookup serves `register` and `definer`.
