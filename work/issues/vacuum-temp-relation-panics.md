---
created-at: 2026-10-26
status: resolved
kind: bug
priority: 692
summary: "vacuum of a temp relation panics in Database::vacuum on a missing map entry, and reindex of one gives a misleading Unsupported"
closed: 2026-10-26
tags:
  - quality
  - engine
---

# vacuum of a temp relation panics with no entry found for key

## Report

The checker resolves `vacuum r` against the schema extended with the transaction's temp relations, so a temp relation reaches `Database::vacuum` (`crates/adtb/src/write/mod.rs:612`, called from `crates/adtb/src/api/command.rs:276`), which indexes `self.rels[&rel]` and `self.indexes[&r.key_index]` and panics.

```adt
temp relation t : User key id
vacuum t
```

Run through `query_text`.

Actual: panic `no entry found for key` at write/mod.rs:612.

Expected: a refusal naming `t` as a temp relation.

`reindex t` on the same temp relation gives `Unsupported` with the message "rebuilding indexes [] of t", which is also the wrong message.

Fix: refuse a temp relation as the target of `vacuum` and `reindex` when checking the text, and have `Database::vacuum` use `self.rels.get` and return an internal error for a relation with no heaps.

## 2026-10-26: done

The ad hoc checker refuses a temp relation as the target of `vacuum` and `reindex` with "`vacuum` takes a stored relation; `t` is a temp relation", whether the same text or an earlier statement declared it. `Database::vacuum` returns `Error::Internal` for a relation with no heaps. Tests in `adtb-lang` `check::tests::adhoc::temp_relations_check` and `adtb-sim/tests/commands.rs`.

## 2026-10-26: resolved

## 2026-10-26: landed

Landed with the ebug track.
