---
title: Why a match with multi-test arms costs exponential work
---
Repository evidence at main eeb77f2f, for `lang/patterns/failure-sharing`.
Ticket: work/issues/match-failure-duplication.md.

## The lowering

`chain` (crates/adtb-lang/src/check/pat.rs:1075) compiles arms last to
first, threading the failure continuation `f` (the arms below). A guard
becomes `If(g, body, f)` (pat.rs:1092); `compile` turns a literal test
into `If(test, k, f)` (pat.rs:1121); `compile_fields` passes the same
`cont.f` to every field subpattern (pat.rs:1152-1170); a nested
constructor test is a `Case` whose non-matching arms are all `f`
(pat.rs:1131-1145). Any arm with two or more failure points (two field
literals, a literal and a guard, a nested constructor and anything)
references `f` twice. `f` holds every later arm, so the tree size is
about 2^n for n such arms.

Unguarded arms are affected too:
`match i | Item { n: 1, name: "a" } -> 1 | ...` duplicates `f`.

Or-pattern alternatives also bind one `Var` at several sites
(pat.rs:1116 inside the loop at pat.rs:1123-1128), against
docs/design/kernel.md §2 ("no two binding sites share one"); the sites are
exclusive branches.

## The arena already shares `f`

`f` is one `Tm`, so the checked arena is linear: measured distinct nodes
grow by about 10 per arm while the tree size doubles per arm (table
below). docs/design/kernel.md §2 already states terms form a DAG and a
consumer must not attach per-evaluation state to a node. The cost comes
from passes that walk the DAG as a tree.

## Which passes do not exploit the sharing

Measured with a scratch harness calling `check_adhoc`, `norm::adhoc`,
`Terms::writes` and `write_depends_on_volatile` (release build). `q` is
`match count(x for x in items) | i if count(y for y in logs) > i -> i |
... | _ -> 0`; `w` the same with arm bodies `insert logs { label: .. }`
into a `generate sequence` relation; `u` the unguarded two-field-literal
match above.

| text | checked distinct / tree | normal distinct / tree | check_adhoc | norm::adhoc | writes | volatile |
| --- | --- | --- | --- | --- | --- | --- |
| q14 | 146 / 2.0e5 | 235 / 2.3e5 | 19 ms | 0.2 ms | 6 ms | 16 ms |
| q18 | 186 / 3.1e6 | 337 / 3.7e6 | 204 ms | 0.3 ms | 70 ms | 168 ms |
| q22 | 226 / 5.0e7 | 455 / 5.9e7 | 2.4 s | 0.5 ms | 0.96 s | 2.4 s |
| w14 | 202 / 2.6e5 | 98 414 / 2.9e5 | 140 ms | 149 ms | 6 µs | 24 ms |
| w18 | 258 / 4.2e6 | 1 573 006 / 4.7e6 | 2.9 s | 2.6 s | 8 µs | 438 ms |
| u20 | 189 / 1.3e7 | 377 / 1.3e7 | 632 ms | 0.4 ms | 242 ms | 883 ms |

- Normaliser, queries: linear. The memo keyed on (arena, `Tm`, type
  arguments, values of free variables) (crates/adtb-lang/src/norm.rs:569,
  :582) shares `f`.
- Normaliser, writes: the memo refuses any result that draws
  (`!self.draws(*o)`, norm.rs:591 and in `Wrap::Memo` norm.rs:787). A
  `Fresh` key fill makes every tail draw, so each path re-normalises `f`
  and the output arena itself is exponential (1.5 M nodes at 18 arms).
  The refusal came with the volatile-hash-cons fix
  (work/issues/volatile-hash-cons.md): two call sites of one inlined
  function body coincide in the memo key, and the memo cannot tell that
  sharing apart from sharing between exclusive branches.
- `check_adhoc` runs `analysis::adhoc_diags`
  (crates/adtb-lang/src/analysis/mod.rs:193), which normalises and then
  calls `write_depends_on_volatile` (analysis/volatile.rs:23), a tree walk
  whose `dirty` test calls `Terms::any` at every `Let`, `If` and `Case`.
  The query `check_adhoc` time is that walk.
- `Terms::any` and `Terms::writes` (crates/adtb-lang/src/kernel.rs:346,
  :359): tree walk, no visited set; called from crates/adtb/src/api/text.rs:119
  on every normal form.
- `reads_only_row` (crates/adtb-lang/src/check/mod.rs:825): tree walk.
- Per a read-only audit of the other consumers: tree walks with no memo in
  `analysis/aggregate.rs`, `translate/walk.rs` `yields`,
  `permission/eval.rs` (abstract evaluation of both branches),
  `adtb/src/plan/lower/mod.rs` `eval` (:612), `spine` (:1026) and
  `delta` (:1362), `adtb/src/exec/code.rs` `expr`/`block` (:619, :588),
  `adtb/src/write/explain.rs` `write_node` (:86) and
  `adtb/src/write/delta/mod.rs` `top`, `nested` (:551, :596) and
  `unfold` (:836). Several of them carry path context: the delta's guards
  along the path, the planner's known constructors, emitted instruction
  blocks per path. Their output is exponential, not only their time.
- Passes that already share: `Totality` (analysis/totality.rs:35),
  `analysis::accept` (analysis/mod.rs:28), fix analyses, `system::program`,
  `translate::Walk::tm`, the planner's `Facts`, `specialise`, `import`,
  `polarity`, delta `scan`, `replace`, `subst`. The reference interpreter
  takes one branch per evaluation and is linear per run.
