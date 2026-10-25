---
title: "Which inline flags a matches pattern accepts"
pick: one
options:
  re2:
    title: "RE2's i, m, s and U, anywhere in the pattern; x and R refused; . excludes a line feed unless s"
  leading-i:
    title: "Only a leading (?i)"
---
Raised by the reference interpreter. prelude.md §4 said a pattern is
case-sensitive unless it begins with `(?i)`, and said nothing of the
other RE2 flags or of whether `.` matches a line feed.

## spec/regex-flags/re2

A pattern sets flags as RE2 does, `(?flags)` for the rest of its group
or `(?flags:re)` for `re`, anywhere in the pattern, and `-` clears
them: `i` compares by simple case folding, `m` lets `^` and `$` match
at line feeds, `s` lets `.` match a line feed, `U` swaps greedy and
lazy repetition. The flags `x` and `R` are a compile error. Without
`s`, `.` matches every scalar value but a line feed.

```adt fragment
matches(t, "^(?i)ab(?-i:C)$")
matches(t, "(?s)a.b")
```

## spec/regex-flags/leading-i

Only a leading `(?i)` is a flag; every other `(?` group is a compile
error. Smaller, but RE2 patterns written elsewhere stop working, and
`.` and line feeds stay unstated.
