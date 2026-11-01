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

`additional-original-cases.txt` records 68 operations from 24 wholly migrated
families and the ordinary Exists/Top paths. Its separate fingerprint preserves
the first manifest
unchanged. Both baseline manifests are checked for required membership;
future additions still need only a runtime case file.

Accepted schema cases may declare `body` and `sexp` to compare the exact
printed checked function body. Accepted schema or ad hoc cases may declare
`constructors`: each line has four tab-separated columns containing the
ordered sum constructor names (separated by `|`), raw constructor index,
constructor name, and ordered field names (separated by commas, empty for
nullary constructors). The driver compares sorted, distinct reachable
constructor tuples. Raw expected indices are parsed as numbers independently
of the checked type. Schema probes require `bodies`, one function name per
line, and may declare `root_ctor` to require a constructor root at that raw
index. Ad hoc probes inspect the sole result. These checks inspect checked
terms; all successful paths also retain the shared normalization invariants.

The existing typed structural assertions load `.probe` files through the
shared `probes` helper. Each file's stem names its assertion entry point;
the roster stores the actual functions. Append blocks with mode `probe`
and a unique local ID to extend a property without changing Rust. The
assertion reads semantic fields through `text`, `number`, `numbers`,
`aggregates`, or `paths`. Unused fields fail after each case. Every selected
case executes once, and failures aggregate with their IDs after all cases.
Unknown property files, duplicate IDs, empty files, and missing original
IDs fail. `probe-original-cases.txt` pins the original 28 typed probe cases
across 25 assertion families. These cases contain 33 source operations;
most inspect one source, while a few compare related lowering forms.

Aggregate expectations use tab-separated operation (`min`, `max`, `avg`)
and nonempty-source Boolean pairs. Index paths have one path per line;
steps are comma-separated `field NAME` or `ctor RAW_INDEX`, and `empty`
represents the empty path. These parse typed expected values independently
of the checked program. Expected text preserves its Unicode bytes.

The generated 4000-field coverage test keeps its generator and 2 MiB stack
harness in Rust. The synthetic shared DAG traversal test keeps kernel
construction and actual node identity in Rust. Every other ordinary source
and expected value in the original remaining checker module cohort is
runtime data. Typed term shape and identity assertion logic stays in Rust.
