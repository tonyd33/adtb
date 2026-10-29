---
title: How a match's failure continuation is shared in kernel terms
pick: some
raised-by: [lang/patterns/guards, lang/patterns/nested, lang/patterns/literals, lang/patterns/record]
options:
  memo:
    title: Keep the lowering; memoise every kernel pass on term identity
    assessment: {grade: hack, why: path-sensitive passes cannot share a result across paths and every future pass must remember to memoise}
    links:
      - {rel: prior-art, to: prior/match-failure-join}
  join:
    title: A kernel join point form, non-recursive and jumped to only in tail position
    assessment: {grade: costly, why: "every consumer of kernel terms gains a form, and set-valued planning and incremental checks need path conditions merged at the join"}
    links:
      - {rel: prior-art, to: prior/match-failure-join}
  fold:
    title: Fold a guard and binder-free tests into one condition; memo or join for the rest
    needs-one-of: [lang/patterns/failure-sharing/memo, lang/patterns/failure-sharing/join]
    assessment: {grade: costly, why: "covers literal and guard arms only; nested constructor tests still need memo or join"}
  retest:
    title: Lower each arm to one condition and one body, retesting the pattern in the body
    excludes: [lang/patterns/failure-sharing/memo, lang/patterns/failure-sharing/join, lang/patterns/failure-sharing/fold]
    assessment: {grade: clean, unverified: true, why: "linear terms with no new kernel form; costs a repeated total test per nested constructor"}
    links:
      - {rel: prior-art, to: prior/match-failure-join}
links:
  - {rel: evidence, to: match-failure-duplication-evidence}
---
The pattern lowering (crates/adtb-lang/src/check/pat.rs `chain`,
`compile`, `compile_fields`) gives every test of an arm the same failure
continuation `F`, the arms below. An arm with two or more tests (a
literal and a guard, two field literals, a nested constructor and
anything) references `F` twice:

```
match y | 1 if g -> e | rest
  =>  If(y == 1, If(g, e, F), F)          -- F = lowering of `rest`
match i | Item { n: 1, name: "a" } -> e | rest
  =>  If(i.n == 1, If(i.name == "a", e, F), F)
```

The arena shares `F` by id, so the checked term is linear, but its tree
is about 2^n for n such arms, and every pass that walks it as a tree is
exponential (notes `match-failure-duplication-evidence`). A write match
whose arms insert into a generated-key relation is exponential in the
normaliser itself: its memo refuses results that draw `Fresh`. Binding
`F` with `Let` does not work for writes: `Let(v, e, w)` evaluates `e`
first and a `Let` whose bound term contains `Fresh` is kept
(docs/design/kernel.md §4.1, §7).

Whatever is chosen must keep `query/native` (one union-index probe routed
by constructor: the per-constructor top-level `Case` that `chain` builds
for a sum scrutinee) and the evaluation order of partial guards
(`types/fault/reorder/ordered-guards`).

## lang/patterns/failure-sharing/memo

No change to the lowering or the kernel. Every pass over kernel terms
memoises on `Tm`, keyed by whatever context it carries, as `Totality`,
`Walk::tm` and the planner's `Facts` already do. Fix list today:
`Terms::any`/`writes` (kernel.rs:346), `write_depends_on_volatile`
(analysis/volatile.rs:23), `reads_only_row` (check/mod.rs:825),
`analysis/aggregate.rs`, `translate/walk.rs` `yields`,
`permission/eval.rs`, `plan/lower/mod.rs` `eval`, `spine`, `delta`,
`exec/code.rs` `expr`/`block`, `write/explain.rs`, `write/delta/mod.rs`
`top`, `nested`, `unfold`; and the normaliser memo must record results
that draw (norm.rs:591, :787).

- Reference interpreter: nothing; it takes one branch per run.
- Planner and executor: the code generator must emit the shared block
  once and branch to it (a label), and the planner must plan a shared
  set-valued tail once although it is reached under different conditions.
- Translation: `Walk::tm` already shares; `yields` needs a memo.
- Oracle: nothing.

## lang/patterns/failure-sharing/join

A kernel form for a local, non-recursive, nullary join point, as GHC's
System FJ `join`/`jump` and OCaml's `Lstaticcatch`/`Lstaticraise`:

```
Join(j, F, M)     -- τ = ty(M); ty(F) a subtype of τ; binds label j in M
Jump(j)           -- any type; only in tail position of M w.r.t. Join(j)
```

Tail positions of `M`: `M` itself, both branches of an `If`, every arm of
a `Case`, the body of a `Let`, `Block` or `LetBang`, and the scope of an
inner `Join`; never a condition, scrutinee, bound term, `For` body,
aggregate body, field or argument. `F`'s free variables are bound
outside the `Join`, so a jump passes no arguments.

Evaluation: `Join(j, F, M)` evaluates `M`; reaching `Jump(j)` evaluates
`F` in the environment of the `Join` and that is the result. `F` is
evaluated once per jump taken and never otherwise, so a `Fresh` in `F`
draws once per failure that reaches it. The match lowers to
`Join(j, F, If(y == 1, If(g, e, Jump(j)), Jump(j)))`.

- Reference interpreter: a new form; jump as a non-local return to the
  `Join`, or a loop that replaces the current term with `F`.
- Normaliser: keep join points join points: a context pushed into `M`
  (for-of-if, `Field` of a branch) is pushed into `F` too. Memo by
  label.
- Planner and executor: `F` planned once; in a set-valued position `F`
  contributes rows under the disjunction of the conditions of the paths
  that jump, which the planner and the incremental delta must compute.
- Translation, permission, polarity, explain, volatile, totality: a new
  case each.
- Oracle: nothing beyond the interpreter.

## lang/patterns/failure-sharing/fold

The task's minimal change: where the guard and the tests bind nothing the
guard needs from a later test, fold them into one condition with the
kernel's short-circuit `and`, which is already `If(a, b, False)`
(check/expr.rs:650-660):

```
| 1 if g -> e            =>  If(If(y == 1, g, False), e, F)
| Item { n: 1, name: "a" } -> e
                         =>  If(If(i.n == 1, i.name == "a", False), e, F)
```

A nested constructor pattern binds its row only inside a `Case`, so it
still duplicates `F` and needs `memo` or `join`.

## lang/patterns/failure-sharing/retest

Every arm lowers to one condition and one body, and `F` appears once:

```
If(test_i, body_i, F)
```

`test_i` is a total Bool term: the pattern's tests, then the guard, in
order, conjoined with the short-circuit `and`. A nested constructor test
is a `Case` returning `False` from the other constructors. `body_i`
destructures the value again to bind the pattern's names, with
`Unreachable` for the constructors `test_i` excluded, and then evaluates
the arm body once:

```
| Shipped { via: Courier c } if c.ok -> e | rest
  =>  If(Case(y.via, Courier c' -> c'.ok, Post _ -> False),
         Case(y.via, Courier c -> e, Post _ -> Unreachable),
         F)
```

The guard is checked under its own binder variables (`c'`), the body
under the arm's (`c`), so each `Var` keeps one binding site. Field
projections and constructor tests are total, so a fault can come only
from the guard, in the same order as today; a `Fresh` in the body draws
once.

- Reference interpreter: nothing.
- Normaliser: nothing; the term is linear and the memo is not needed.
- Planner and executor: nothing new. The top-level per-constructor `Case`
  of `chain` is unchanged, so `query/native` routing is unchanged; a
  nested constructor test is evaluated twice (once to test, once to
  bind) unless the planner shares it.
- Translation: nothing.
- Oracle: nothing.
- docs/design/kernel.md: `Unreachable` is "a position exhaustiveness
  proves no evaluation reaches"; it becomes "a position the checker
  proves no evaluation reaches".

Prior art: backtracking automata (Augustsson 1985; Maranget 2008 §1),
linear in code size at the price of retesting subterms.
