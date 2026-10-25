---
created-at: 2026-10-25
status: resolved
kind: bug
priority: 291
summary: "ten spec gaps in collation names, Unicode versions, regex flags, textual forms and zone edges"
closed: 2026-10-25
tags:
  - spec
---

# Bug: Text, collation and zone gaps in the spec

## Report

Found implementing the Unicode text functions and the textual syntax
in `adtb-reference`. My rulings, one per gap:

1. Collation versions are named by CLDR release: `@cldr-49`, not
   `@icu-74`. Fix `catalog.md:617`, `model.md:368` and the prelude prose.
   Those two corpus blocks fail until then.
2. A well-formed locale with no tailoring falls back to root order, as
   CLDR does. `sys::collations` lists root and the tailored locales.
3. A database records one Unicode version, and every text function
   follows it, `\p{..}` in `matches` included.
4. `matches` accepts RE2's flags `i`, `m`, `s` and `U` anywhere, and
   refuses `x` and `R`. `.` excludes `\n` unless `(?s)` is set.
5. `SortKey` has a textual form, `0x..` hex as `Bytes`. Add it to
   interop §1.3.
6. A `Span` prints and reads as `span(a, b)`.
7. Zone names are matched by exact spelling and kept as written.
   `europe/lisbon` is `UnknownZone`.
8. `to_instant` and `in_zone` may fault `ArithmeticOverflow` at the
   edges of the instant range.
9. `in_zone` then `to_instant` gives the earlier instant for the later
   half of an overlap. State it.
10. Printing writes a control character as `\u{..}`; reading accepts
    either form.

Point 3 means the reference needs a `regex-syntax` at the same Unicode
version as ICU4X. Until then the reference deviates there; record that
in `docs/design/oracle.md`.

## 2026-10-25: resolved

In the spec with `docs: record the spec2 rulings on text, zones, dumps, temp relations and constraint names`. Point `spec2`, for Tony to adopt.
