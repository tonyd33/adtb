---
created-at: 2026-10-29
status: resolved
kind: feature
priority: 420
summary: Implement the grant rules spec revision 30 states
closed: 2026-10-30
---

# Feature: Implement the grant rules spec revision 30 states

## Request

The spec landed at a478f4ba (rulings in design-space point current) ahead of the code. Code gaps,
checked against security.md 3.2, 3.3, 3.6 and catalog.md 2.4, 2.6:

- Grant and revoke accept a row target text: `frozen (...)`, `C.f`, `except`, and a family target
  `FAMILY::q at v (params) -> type`. A revoke resolves to row form, then compares exactly. Three
  widenings: no field list, no `except`, a family revoke with no signature.
- Rows are keyed by grantor. A revoke deletes the issuing role rows, the schema owner deletes
  all, and a revoke that orphans rows granted under the option is refused. An operator revoke
  acts as the schema owner.
- Constructors in `.(C | D)` and `except` pairs print in id order.
- The logged grant privilege list: bookmark grant-privilege-clauses holds the spec half
  (`ClauseBody.Privilege`, constructor id 472, fields 569 and 570). Rebase and land it with
  catalog.adt and ids.rs declaring the constructor, or spec_corpus fails.
- `AddDefault` names every constructor by `Member` clauses.

Separate tickets: empty-revoke-refused, carry-keeps-empty-grant,
default-applies-to-later-constructor, sync-call-skips-admission. Family call grants themselves are
in roles-and-grants.
## Proposal

## Open questions

## 2026-10-30: landed

Landed as 0c1fc305. Full check passed: 2923 tests, 7 skipped; campaign 52. An existing log with an
empty revoke now fails to open, and an old `version_hash` is not recomputed; both accepted before
release. A capability is given by a `schema` holder or a holder of it with grant option.

## 2026-10-30: resolved
