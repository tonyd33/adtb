---
name: quality-review
description: Review the adtb changes since the last quality review (`reviewed..@`, or a given jj range) for correctness, duplication, misplaced responsibility, error handling, test strength and the house comment rules, and file one tix ticket tagged quality per real finding. Use for the periodic quality review, "review since reviewed", or a quality pass over a range. Never edits code.
---

# Quality review

Review a range of jj changes and turn each real problem into a ticket.
Never edit code and never move the `reviewed` bookmark; the lead moves it
after reading your report.

## Scope

The range is `reviewed..@` unless the caller gives one. Work from the
repository root.

If `reviewed` is missing, follow
[the review-baseline setup](../../../docs/development.md#moving-work-between-machines)
or ask the caller for an explicit range.

Start with:

```
code-commits 'reviewed..@'
quality-metrics 'reviewed..@'
jj diff -r 'reviewed..@' --stat
```

`code-commits` (`work/tools/code-commits RANGE` outside the dev shell)
lists the commits that change a path outside docs/, work/ and Markdown
files. Review those. Skip the others, which only file tickets or edit
prose; the design-contract spot-check below still reads docs/design/.

`quality-metrics` (or `work/tools/quality-metrics.sh RANGE` outside the
dev shell) prints the changed files, new or changed functions over 100
lines, new `#[expect]`s, panic/unwrap/expect counts per crate and new
`pub` items. Use it to choose where to look first.

Then list the open quality tickets, so you do not file what one covers:

```
nix run github:tonyd33/tix -- -C work/issues list --tag quality
```

## Reading

Read the diff one file at a time: `jj diff -r RANGE -- PATH`. For the
code around a hunk, use your harness's search and file-reading tools to
locate the enclosing function and read it whole. For long `docs/` files,
search the headings and read the relevant section. If dedicated tools
are unavailable, use `rg -n` and a bounded file read in the shell.

## What to look for

- Correctness: a branch that mishandles a case, an off-by-one, a latch
  held across a call that can block, state left half-updated on an
  error path.
- Duplication: a helper that repeats one that exists. Grep for the
  existing one before calling it a duplicate.
- Misplaced responsibility: a crate reaching into another's internals
  instead of the contract in its `docs/design/` file; a rule stated in a
  tier other than the lowest one that can observe it.
- Error handling: a discarded `Result`; an error flattened to a string;
  `unwrap` or `expect` on input that can fail; a panic that does not
  name the invariant it guards.
- Test strength: a test that asserts only that nothing failed instead of
  the result; a seed loop whose assertion message lacks the seed; a
  feature whose failing cases are missing from its tests.
- House comment rules: no em dashes; no spec section numbers, rule ids,
  ticket names or file citations; no comment restating the code; doc
  comments in the imperative or "Returns ..."; no markdown bold.
- Each new `#[expect]`: its reason states what would have to change for
  the expectation to go.

Spot-check one `docs/design/` contract the range touches: pick the
design file of a subsystem the diff changes, read the section for the
changed call, and check the code honours its preconditions,
postconditions and invariants.

Skip what clippy, rustfmt or `size-check` already enforce.

## Filing

One ticket per real finding, skipping any an open `quality` ticket
already covers:

```
nix run github:tonyd33/tix -- -C work/issues new STEM --kind bug --summary "..."
nix run github:tonyd33/tix -- -C work/issues set STEM tags=quality,engine
```

Name the stem for the gap. Follow the format in `work/issues/README.md`:
a bug is titled with its symptom, a feature `# Feature: <what>`, and
`## Report` states the problem concretely with `path:line` at the
range's head, a repro where one applies, and the fix direction. Set the
priority by the README's scheme. Run `tix check` after the last ticket.

## Report

List each ticket filed as its stem and summary, the findings you skipped
because an open ticket covers them, and the design contract you
checked with its result.
