# Checker runtime cases

The three unit-test groups in `src/check/tests/runtime.rs` discover every
`.case` file here at runtime. Adding a file or changing its data requires
no Rust change or relink. `running.schema` is the shared schema, read at
runtime by the existing checker helpers.

A file contains one or more blocks:

```text
=== 001 adhoc
--- text
1 + 1.5
--- expect
accept
--- type
Decimal 1
```

The header gives the file-local subcase ID and mode (`schema`, `adhoc`,
`query`). Required fields are `text` and `expect` (`accept`, `refuse`). A
refusal requires `phase` (`schema`, `check`, `adhoc`, `query`) and `errors`,
containing the exact diagnostic, including newlines. Separate diagnostics
with a line containing `::: diagnostic`; comparison preserves multiplicity
and ignores diagnostic order. An accepted ad hoc case may declare `type`
for the exact inferred type of its sole result.

Ad hoc cases may declare `params`, one tab-separated name/type pair per
line. These descriptors are resolved externally through the existing helper,
then passed to the checker. An ad hoc case may instead declare `extra`, a
schema addition checked before the ad hoc text. Query cases may declare
`extra` as well. Unknown, repeated and unused fields fail.

Every field ends with one delimiter newline. Additional trailing newlines
belong to its value and are preserved. The markers `=== ` and a newline
followed by `--- ` are reserved. Files use UTF-8 and LF newlines.

`original-cases.txt` records all 396 subcase IDs from the 108 migrated Rust
test families. Its fingerprint is pinned in the shared driver. Every listed
ID must be discovered, so removing an original case or subcase fails even
when another one is added. New cases need no manifest entry. Each mode
accounts for its selected subcases and aggregates failures after running
all of them. Successful paths retain checked and normalized binding-site
checks over the original complete root sets.
