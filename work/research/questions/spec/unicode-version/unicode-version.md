---
title: "Which Unicode version the regular expressions of matches follow"
pick: one
options:
  one:
    title: "The database's one recorded Unicode version, as every other text function"
  own:
    title: "The regular expression engine's own tables, unstated"
---
Raised by the reference interpreter. model.md §3.5 records one Unicode
version for normalization, case mapping and segmentation; prelude.md
§4 said `\p{..}` classes follow it, but nothing said every text
function does.

## spec/unicode-version/one

A database records one Unicode version, and every text function
follows it: normalization, case mapping, folding, segmentation,
collation's character data and the classes of `matches`.

```adt fragment
matches(t, "^\\p{L}+$")   -- letters of the recorded Unicode version
```

An implementation must then ship regular expression tables of the same
Unicode version as its other text data.

## spec/unicode-version/own

`matches` follows its library's tables. Easier to build, but a stored
constraint over `matches` can change verdict when the engine upgrades
its regular expression library, the same text is a letter to `upper`
and not to `\p{L}`, and moving the Unicode version would no longer be
one schema operator.
