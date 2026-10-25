---
created-at: 2026-10-25
status: open
kind: feature
priority: 210
summary: "docs/design/kernel.md states no contract for the kernel forms of writes and transact blocks"
depends-on: [reference-interpreter-queries]
tags: [design]
---

# Feature: kernel contract for writes and blocks

## Report

`docs/design/kernel.md` has no section on writes. `adtb-lang` represents
writes and `transact` blocks by its own kernel forms, not by the write
terms of `docs/spec/semantics.md` §2.4, and the reference interpreter
needs their meaning stated. The contract below was drafted while the
writes checker landed. Merge it into `docs/design/kernel.md` as section
3 once `reference-interpreter-queries.md` has written sections 1 and 2.

The draft, verbatim except that its links are rewritten relative to
this directory; they are `../spec/` again in `docs/design/kernel.md`:

````markdown
## 3. Writes and blocks

`adtb-lang` represents writes and `transact` blocks by its own kernel
forms, not by the write terms of
[../spec/semantics.md §2.4](../../docs/spec/semantics.md#24-writes-and-statements).
The obligation is the meaning: every term `adtb-lang` produces denotes
the set of effects (`ins`, `del`, `rep`, `kill`) and the result that the
equations of [../spec/semantics.md §6](../../docs/spec/semantics.md#6-writes)
give the desugaring of
[§10.3](../../docs/spec/semantics.md#103-writes) for the surface program it came
from. Each form below is stated against that desugaring.

### 3.1 Types and evaluation

- A write is a term of type `Change`. `Skip`, `Insert`, `Delete`,
  `Replace`, `Upsert`, `Rejects`, `Par` and `Fail` are writes. `If`,
  `Case`, `Let` and `Unreachable` at type `Change` are the write-level
  `if`, `case` and `let` of the calculus; `Let(v, e, w)` evaluates `e`
  once and evaluates `w` with `v` bound.
- Every read inside a write reads the statement's snapshot `D`. No
  write sees another write's effects, except through a variable bound
  by `Let` before a `Block` or by `LetBang` (§3.4).
- A write term is evaluated once per statement unless it sits under a
  `For`, a `Case` arm or an `If` branch. `Fresh(g, r)` draws a new value
  each time it is evaluated, so the interpreter must evaluate the rows
  of an `Insert`, the value bound by a `Let` and the new-value function
  of a `Replace` exactly as often as the term structure says, and no
  more.
- Fills are explicit. A row handed to `Insert`, `Upsert` or a
  `Replace` function gives every field: a field the source omitted is
  `Fresh(g, r)` for a generator, the elaborated default expression, or,
  in a new value, `Field(old, f)` for the key and header fields carried
  from the old row. The interpreter applies no fill of its own.

### 3.2 Forms

| Form | Fields and invariants | Meaning | Replaces in §10.3 |
| --- | --- | --- | --- |
| `Insert(r, e)` | `e : Set T`, `T` a row type of `r` | `{ ins(r, v) \| v ∈ e }` | `insert r {fill_r(...)}`, `insert r (for ξ in q do {fill_r(ξ)})` |
| `Delete(r, e)` | `e : Set T`, `T` a row type of `r` | `{ del(r, v) \| v ∈ e ∩ D(r) }` | `delete r (for x in R do if p then {x} else {})`; `delete x` is `Delete(r, {x})`, `delete x where p` is `Delete(r, if p then {x} else {})` |
| `Replace(r, e, f)` | `e : Set T` of rows of `r`; `f = Lam(x, n)`, `f : T -> T_r` | for each `v ∈ e`: `KeyChanged` if `key_r(f v) ≠ key_r(v)`; nothing if `f v = v` or `v ∉ D(r)`; `rep(r, v, f v)` otherwise | `for x in R do if p then replace r x e° else skip`, with `e` the filtered rows and `f` the completed new value `e°` |
| `Upsert { rel, rows, on, old, new, otherwise, single }` | `rows : Set T`, filled; `on` is `Key` or `Unique(i)`, `i` an index of `rel` with `unique`; `old : T_rel` restricted to the constructor set of `on` (every constructor for `Key`); `new : T`; `otherwise : Change` | for each `n ∈ rows`, `O` = the rows `o` of `rel.S` in `D` whose `on` values equal `n`'s: `ins(rel, n)` if `O` is empty, else the effects of `otherwise` with `old ↦ o`, `new ↦ n`, for each `o ∈ O` | the `let! ξ_V … for new in ξ_V do … if empty(ξ_O) then insert … else for old in ξ_O do w` of §10.3 |
| `Rejects { into, insert }` | `insert` is an `Insert` or `Upsert` whose rows are `For(x, Rel(r₀, S), body)`; `into` holds `RejectedRow` | evaluates `body` per record `p` of the source: rows of records that compute are the inner form's rows; a record whose computation reaches `Reject(m)` or does not convert contributes `ins(into, { load: ℓ, line, source, raw, reason: m })` instead; `ℓ` is one fresh value per statement | `insert r (for x in e₀ do e) rejects into j` (§6.7) |
| `Par(ws)` | each `w : Change` | the union of the effects of every `w`; order chooses neither the change set nor which abort is the outcome | `w₁ ∥ … ∥ wₙ` |
| `Fail(t)` | `t : Text` | rejects the statement with `UserError(t)`; not a fault | `fail e` |
| `Reject(t)` | `t : Text`, any type; only inside the rows of an `Insert` or `Upsert` | the record being computed is rejected with message `t`; without an enclosing `Rejects` the statement fails with `BadInput` | `reject e` |

A `Case` over writes has one arm per constructor of its scrutinee's
constructor set; an arm no evaluation reaches is `Unreachable` at
`Change`.

### 3.3 Blocks

| Form | Fields and invariants | Meaning |
| --- | --- | --- |
| `Transact(b, o)` | `b` a block term; its type is the block's result, `()` when the block has no `return` | evaluates `b` once against the snapshot to effects `E` and a result; `E` with its derived delete actions is the statement's change set (§6.6); `o` is the block's options |
| `Block(w, k)` | `w : Change`; `k` a block term or the result | `w`'s effects join the statement's; returns `k`'s value |
| `Let(x, e, k)` in a block | `e` a query or a row value | `let! x = e in k`: `e` evaluated once, in the snapshot |
| `LetBang(x, w, k)` | `w` a `Replace`, `Upsert` or `Rejects` | `w`'s effects join the statement's and `x` is bound as §3.4 states |

`TxnOptions`: `retry` is the number of reruns after the first attempt
on a serialization failure; `read_only` and `deferrable` as the block
declares them (a `read_only` block contains no write, and declares no
`durability`); `durability` is `Local` or `Quorum` when declared.

A surface block `transact let x₁ = e₁ … w … return e` is a right-nested
chain of `Let`, `Block` and `LetBang` ending in the result term, inside
one `Transact`. It denotes the same effects and result as the
`let! x₁ = e₁ in … (w₁ ∥ … ∥ wₖ, e)` of §10.3, because every item reads
the snapshot and effects are a set.

A registered transaction is `Lam(x₁, … Lam(xₙ, Transact(b, o)))`, one
`Lam` over unit when it has no parameters.

### 3.4 What a `let` binds

- `let x = insert ...` is `Let(x, rows, Block(Insert(r, x'), k))`, where
  `x'` is `{x}` for the single-row form and `x` for the `from` form, so
  the inserted rows, with every fresh value drawn, are the rows `x`
  names.
- `let x = delete ...` is `Let(x, rows, Block(Delete(r, x), k))`: `x`
  is the rows the delete names, as they are in the snapshot, not the
  rows its delete actions derive.
- `let x = update ...` is `LetBang(x, Replace(r, e, f), k)`: `x` is
  `{ f v | v ∈ e }`, the new values with fills applied.
- `let x = <insert> rejects into j` is `LetBang(x, Rejects { .. }, k)`:
  `x` is the rows inserted into the target, without the rejected
  records.
- `let x = <upsert>` is `LetBang(x, Upsert { .. }, k)`: per proposed
  row `n`, the binding is `n` when it is inserted; otherwise the row
  with `old`'s key among the effects of `otherwise` (an `ins` or the
  new value of a `rep`) when there is one, and `old` when there is
  none. `x` is that one row when `single`, the set of them otherwise.
  This is `only (U (B(E_w, {old})))` and `U (B(E_w, {old}))` of §10.3.
  The checker refuses the `let` when `otherwise` can delete `old`,
  directly or through `cascade`, so the binding is defined.

A nested `key (insert r { ... })` is `Let(ξ, row, Par[Insert(r, {ξ}),
w])` around the innermost write `w` that holds it, or `Let(ξ, row,
Block(Insert(r, {ξ}), k))` around a block item. `ξ`'s row is evaluated
once per statement: the checker admits the form only where that holds.

### 3.5 Snapshot rows

The rows `Delete` and `Replace` take are rows of the relation in the
snapshot: generator binders, `old`, a block `Let` bound to a row read
from the snapshot, and match binders over one. A row bound from a
write is never one of them. An interpreter may still meet a value that
is not in `D(r)` there (a pre-state row deleted twice, say); `Delete`
and `Replace` then derive nothing for it, as the table states.

### 3.6 Delete actions

`Program.retags` maps each relation whose type declares an `on delete
retag D { ... }` to `Lam(x, D { ... })`: the new value of a referrer
`x` that the action retags, its key and header fields `Field(x, f)`,
the payload's literals, and the other own fields of `D` filled as an
insert into `r.D` fills them. Every retag action of one type is one
action, so one term per relation suffices. Which referrers it reaches,
and how `clear` and `cascade` combine with it, is the interpreter's,
by [../spec/semantics.md §6.4](../../docs/spec/semantics.md#64-delete-actions).
````
