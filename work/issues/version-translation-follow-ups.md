---
created-at: 2026-10-29
status: open
kind: bug
priority: 870
summary: "translation blames an unrelated operator for at blocks and file reads, explain skips translated bundles, and the pinned law cannot add fields to nested sums"
---

# Bug: version translation follow-ups

depends on: version-translation
needed by: none

## Report

Found by the second review of version-translation (work/briefs/review-version-translation2.md, local) and left out of its fix scope:

1. When a refusal matches no operator, `blame` names the first operator of the last migration. It is reachable through `at` blocks, file reads, a default with no term, and a function with no catalog id. In `recheck`, a live bundle that uses `at` or reads a file blocks every later migration and names that migration's unrelated first operator. No test reaches the fallback.
2. `explained` skips bundles registered through translation.
3. A `let` variable inserted into two relations takes only the last write site's default (translate walk.rs, `mark`).
4. A key whose own type gained a field could merge rows the pin keeps distinct.
5. The pinned law in crates/adtb-testkit/tests/differential.rs cannot witness an aggregate over values of a type that gained a field: the walk generator adds fields only to relation types, never to a nested sum.
6. A deep law sweep in one run hits nextest's 60 s timeout after 8 to 10 walks; the law takes 11.2 s alone.
7. A nested constructor built inside a write after it gained a field is refused "outside a write".
8. When the catalog itself refuses a migration (for example "holds rows; it cannot be disabled"), `SchemaRefused.operator` is empty.
9. docs/spec/catalog.md's errors table row for `BundleExpired` says an expired or superseded bundle "registered again fails the checks of a re-registration", which reads as every failing check. Section 4 of the same file (about line 599) and evolution.md 7.3 say a stale one is refused `BundleStale` and one with no translation `BundleExpired`; the code follows that. Lead ruling: the table row is imprecise, not the code. Reword it through the spec-revision workflow.
10. A retired bundle registered again with no translation is refused `Untranslatable`, because `translations` runs before `bundle::admitted` (crates/adtb/src/api/registered.rs about lines 296-303); the stale path runs `admitted` first. Run `admitted` first, and refuse a retired bundle `Retired` as the errors table says (main refused it `Invalid`). Witness: open `[V1, ADD_HASH]` with web-1.0, `retire bundle web-1.0`, open `[V1, ADD_HASH, DROP_HASH]` with web-1.0.
11. `rows_of` over-refuses row sets that are distinct by key (`let`-bound, `top`-wrapped); the pinned law never uses `hide new constructors` and does not count the bundles it drops as `BundleStale`.

## Fix

Name the construct the query cannot cross (`at`, a file read) instead of an operator for 1, with a test. Show translated bundles in `explained`. Rebuild per write site for 3. Refuse or translate 4 by the key's own fields. Extend the walk generator for 5 and split the law per seed for 6. Translate 7 through `missing`. Name the refused operator in 8.

## 2026-10-29: from the quality review

- When `Pinned::new` fails, the refusal's query name is empty (crates/adtb/src/api/registered.rs about line 559), so it names `web-1.0::`.
- The `first` closure in `recheck` repeats `translate::operator_text`.
