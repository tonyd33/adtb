---
title: "Whether the Perl classes of a matches pattern are ASCII or Unicode"
pick: one
options:
  unicode:
    title: "Unicode: \\d, \\w, \\s and \\b follow the recorded Unicode version"
  ascii:
    title: "ASCII, as in RE2"
---
Raised by the reference interpreter. prelude.md §4 says `matches` uses
the RE2 syntax, and in RE2 `\d`, `\w`, `\s` and `\b` match ASCII only.
Both the reference and the executor match `\d` against `٣`.

## spec/regex-classes/unicode

`\d` is a decimal digit (`\p{Nd}`), `\s` a white space character
(`\p{White_Space}`), `\w` a word character of Unicode Technical
Standard #18 (letters, marks, decimal digits, connector punctuation
and join controls), and `\b` a boundary between a `\w` and a scalar
value that is not one. All follow the recorded Unicode version, as
`\p{..}` does. `matches` works over Unicode scalar values, so its
classes do too.

```adt fragment
matches("٣", "^\\d$")   -- true
```

## spec/regex-classes/ascii

`\d` is `[0-9]`, `\s` `[\t\n\f\r ]`, `\w` `[0-9A-Za-z_]`, as RE2. A
pattern written for RE2 means the same, but `\w` stops at `é`.
