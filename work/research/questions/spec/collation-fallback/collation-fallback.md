---
title: "What a well-formed locale with no tailoring of its own collates by"
pick: one
options:
  root:
    title: "Root order, as CLDR falls back; sys::collations lists root and the tailored locales"
  refuse:
    title: "A compile error: only listed tailored locales are names"
---
Raised by the reference interpreter. A name such as `"en-AU@cldr-49"`
is a well-formed locale for which CLDR has no tailoring. The spec said
only that an unknown collation is a compile error.

## spec/collation-fallback/root

The locale collates in the order of its nearest tailored parent, which
for most locales is the root collation (`und@cldr-49`), as CLDR's
fallback does. `sys::collations` lists the root collation and each
tailored locale; a name it does not list may still be accepted.

```adt fragment
collate(t, "en-AU@cldr-49")   -- the order of und@cldr-49
```

## spec/collation-fallback/refuse

Only the root and the tailored locales are names; every other locale
is a compile error. The list in `sys::collations` is then exact, but a
program must know which locales CLDR happens to tailor, and a later
CLDR release that adds a tailoring turns a refused name into a valid
one.
