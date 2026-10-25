# Kernel terms

`adtb-lang` provides typed kernel terms to three consumers: the engine
`adtb`, the reference interpreter `adtb-reference`
([oracle.md](oracle.md)), and code generation tools for host languages.
The kernel calculus itself is
[../spec/semantics.md §2](../spec/semantics.md#2-kernel-syntax); this
file states how `adtb-lang` represents it (`kernel.rs`) and what a
consumer may rely on.

## 1. What a consumer receives

The engine and the reference interpreter both consume typed kernel
terms and share nothing else.

| Entry point | Returns |
| --- | --- |
| `check_schema(schema)` | `Program`: one arena holding the term of every function, constraint, check, newtype domain and retag delete action (§4.6) of the schema |
| `check_adhoc(schema, items, params)` | `Adhoc`: an arena with one term per statement, and the variable of each ad hoc parameter |
| `check_query(schema, module, q)` | `Query`: an arena with the term of one registered query |

A term is a `Tm`, an index into its arena's `nodes`; each node has a
`kind` and a `ty`. A `Call` names a function by `FnId`, and the
function's term is `Program.functions[f]`, in the `Program` arena,
whichever arena the call is in. `Program.functions[f]` is `None` for a
Wasm or host function.

Postconditions on every term `adtb-lang` returns:

- It is well typed: each node's `ty` is the type of its kind under §3,
  given its children's types.
- It is closed. Its free variables are the parameters of an `Adhoc`,
  listed in `Adhoc.params`, each of a first-order type. A function or
  registered query is a closed chain of one `Lam` per parameter, or
  of one `Lam` over a `Unit` variable when it has none.
- Its constructor sets are listed: every `Rel`, `Transition`,
  `Restrict`, row type and key type carries the sorted, distinct
  constructor indices it ranges over, a named set already replaced by
  its members. `None` on a `Rel`, `Transition`, row type or key type
  means every constructor of the sum.
- Every `Case` names each constructor of its scrutinee's constructor
  set exactly once.
- Every call of a generic function is instantiated: `Call.type_args`
  holds the type arguments
  [../spec/typing.md §2.2](../spec/typing.md#22-checked-and-synthesized-forms)
  solves.
- Every `Top`, and every `Present` with a `take` or an `after`, orders
  by keys injective on its source.

Not guaranteed:

- A term the checker returns is not normalised
  ([../spec/semantics.md §5.11](../spec/semantics.md#511-normalisation)).
  Calls of schema functions remain `Call` nodes, lambdas and
  applications remain, and a `let` remains whatever its bound
  expression.
- Totality is not marked on nodes.
- Stratification and boundedness of a `fix`
  ([../spec/semantics.md §5.10](../spec/semantics.md#510-fixpoints))
  are not checked.

`adtb-lang` owns normalisation. It provides the planner normalised
terms: no λ and no application, and a `let` only where its bound
expression is partial. The checker's terms above are its input.

TODO: the normalisation entry point, its output arena, and which of the guarantees above it adds are undecided.

## 2. Variables and sharing

A `Var` is an index into its arena's `vars`, each with a name and a
type. Every binding site binds its own `Var`; no two binding sites
share one.

| Form | Binds | In scope in |
| --- | --- | --- |
| `Let(x, e, b)` | `x : ty(e)` | `b` |
| `Lam(x, b)` | `x` at its `VarInfo` type | `b` |
| `Case { arms }` | each arm's variable, at the row type of its one constructor | that arm's body |
| `For(x, src, b)` | `x`, an element of `src` | `b` |
| `Agg`, `Group`, `Top`, `Present` | `var`, an element of `source` | `body`, `by` or the keys |
| `Fix(binds, b)` | each `X_i : Set τ_i` | every bound body and `b` |

Terms form a DAG: one `Tm` may be a child of several nodes, and one
subterm, binders included, may be evaluated in many environments. A
consumer must not attach per-evaluation state to a node or a variable.

`Terms::children` lists a node's children in evaluation order, except
that `Top` lists its count, then its source, then its keys.

## 3. Kinds and their typing invariants

`τ` is the node's `ty`; `ty(e)` is a child's. Types are `types::Ty`:
`Data { def, args, ctors }` is `N τ̄ . S` (`ctors: None` for every
constructor), `Record` an anonymous record with fields sorted by name,
`Sum` an anonymous sum with constructors in written order, `Set` and
`NonEmpty` are `Set` and `Set⁺`, `Key { rel, ctors }` is `Key r.S`,
`Unit` is the empty record `{}`, `Change` a write (§4). `Ty::Group` never
occurs on a node.

`Param(i)` occurs only in the terms of a generic function. It stands
for the `i`-th type argument of the `Call` being evaluated; a consumer
substitutes the call's `type_args`, themselves substituted by the
caller's, before reading a type that may hold one.

| Kind | Invariant |
| --- | --- |
| `Var(x)` | `τ` is `x`'s type |
| `Const(Int(i))` | `τ` is an integer type, or a `Param` whose argument is any numeric type, the constant then being `i` at that type |
| `Const(Decimal(d))` | `τ` is `Decimal s`; the value is `d / 10^s` |
| `Const(Float(f))` | `τ` is `F32` or `F64`; for `F32`, `f` is already rounded to `F32` |
| `Const(Text(t))` | `τ` is `Text`; `t` is NFC |
| `Const(Duration(d))` | `τ` is `Duration`; `d` is microseconds |
| `Prim(p, args)` | as §5 states for `p` |
| `Record(fs)` | `τ` is a `Record` with the same labels; fields sorted by name |
| `Field(e, l)` | `ty(e)` is a record, or a row type every constructor of which has `l`; `τ` is `l`'s type |
| `Ctor { ctor, fields }` | `τ` is a declared sum (`ctors` is `Some([ctor])`, or `None` when the sum has one constructor), an anonymous `Sum`, or `Tag T`; `ctor` indexes the declaration order, the written order, or `T`'s constructors; every field of the constructor, header fields included, is given once, sorted by name; a `Tag` constructor has none |
| `Case { scrutinee, arms }` | `ty(scrutinee)` is a sum or row type with constructor set `S`; one arm per constructor of `S`, ascending; each arm's body is a subtype of `τ` |
| `If(c, a, b)` | `ty(c)` is `Bool`; `a` and `b` are subtypes of `τ` |
| `Let(x, e, b)` | `τ = ty(b)` |
| `Lam(x, b)` | `τ = Fn(ty(x), ty(b))`; one parameter |
| `App(f, a)` | `ty(f) = Fn(σ, τ)` with `ty(a)` a subtype of `σ`; one argument |
| `Call { func, type_args, args }` | `args` holds exactly the function's declared parameters, possibly none; `τ` is its result at `type_args` |
| `Caller` | `τ` is the type `Principal` names |
| `Key(e)` | `ty(e)` is a row type of a declared sum whose relation `r` is statically known; `τ = Key r.S`, `S` the row type's set |
| `KeyOf(r, e)` | `ty(e)` is `r`'s key value type (§6); `τ = Key r` |
| `Deref { key, reference }` | `ty(key) = Key r.S`; `τ` is `r`'s row type at `S`; `reference` holds when `key` is a reference field of a row, so a valid state has the row |
| `Tag(e)` | `ty(e)` is a declared sum or a row type of one; `τ = Tag N` |
| `Try(e)` | `τ = Result ty(e) Fault` |
| `Rel(r, S)` | `τ = Set` of `r`'s row type at `S` |
| `Transition(t, r, S)` | as `Rel`; only in a constraint body |
| `Restrict(e, S)` | `ty(e)` is a set of a sum; `τ` is that set restricted to `S` |
| `EmptySet`, `Singleton(e)`, `Union(a, b)` | `τ` is a set type; elements are subtypes of its element type |
| `For(x, src, b)` | `ty(src)` is a set of `ty(x)`; `τ = ty(b)`, a set |
| `IsEmpty(e)`, `Exists(e)` | `ty(e)` is a set; `τ = Bool` |
| `Agg { op, var, source, body }` | `ty(source)` is a set, `Set` even when the range is nonempty; `τ` is `I64` for `count` and `ty(body)` for `sum`; for `min`, `max` and `avg` it is `ty(body)` exactly when the range is nonempty (a single generator over a group, with no filter), else `Option ty(body)` |
| `Group { var, source, by }` | `τ = Set { key: ty(by), rows: Set⁺ ty(var) }` |
| `Top { count, keys, var, source }` | `ty(count) = I64`; each key is over `var`, ordered, `true` for `desc`; `τ = ty(source)` |
| `Present { var, source, keys, take, after }` | the final result of an ad hoc statement or a registered query body, nested nowhere else; `take : I64`; `after : Option K` for one key of type `K`, or an `Option` of a record with one field per key, whose fields in name order match the keys in order; `τ = ty(source)`, delivered as a sequence |
| `Fix(binds, b)` | each body has its binder's type; `τ = ty(b)` |
| `At { instant, point, body }` | `ty(point)` is `Instant`, or `Position` when `instant` is false; `τ = ty(body)` |
| `Unreachable` | a position exhaustiveness proves no evaluation reaches, at any type; reaching it is a broken invariant |
| writes and blocks | as §4 states |

A `Record` or `Ctor` evaluates its fields in the order the node lists
them, which is name order.

TODO: whether record and constructor fields are evaluated in their written order, as [../spec/semantics.md §5.1](../spec/semantics.md#51-scalar-forms) has it, is undecided.

TODO: the constructor indices of an anonymous sum follow its written order, while two spellings in different orders are one type; how a consumer identifies their constructors is undecided.

## 4. Writes and blocks

`adtb-lang` represents writes and `transact` blocks by its own kernel
forms, not by the write terms of
[../spec/semantics.md §2.4](../spec/semantics.md#24-writes-and-statements).
The obligation is the meaning: every term `adtb-lang` produces denotes
the set of effects (`ins`, `del`, `rep`, `kill`) and the result that the
equations of [../spec/semantics.md §6](../spec/semantics.md#6-writes)
give the desugaring of
[§10.3](../spec/semantics.md#103-writes) for the surface program it came
from. Each form below is stated against that desugaring.

### 4.1 Types and evaluation

- A write is a term of type `Change`. `Skip`, `Insert`, `Delete`,
  `Replace`, `Upsert`, `Rejects`, `Par` and `Fail` are writes. `If`,
  `Case`, `Let` and `Unreachable` at type `Change` are the write-level
  `if`, `case` and `let` of the calculus; `Let(v, e, w)` evaluates `e`
  once and evaluates `w` with `v` bound.
- Every read inside a write reads the statement's snapshot `D`. No
  write sees another write's effects, except through a variable bound
  by `Let` before a `Block` or by `LetBang` (§4.4).
- A write term is evaluated once per statement unless it sits under a
  `For`, a `Case` arm or an `If` branch. `Fresh(g, r)` draws a new value
  each time it is evaluated, so the interpreter must evaluate the rows
  of an `Insert`, the value bound by a `Let` and the new-value function
  of a `Replace` exactly as often as the term structure says, and no
  more.
- `Fresh(g, r)` occurs only in fills. `Only` is never produced: a
  single-row upsert's binding is `Upsert { single: true, .. }`.
- Fills are explicit. A row handed to `Insert`, `Upsert` or a
  `Replace` function gives every field: a field the source omitted is
  `Fresh(g, r)` for a generator, the elaborated default expression, or,
  in a new value, `Field(old, f)` for the key and header fields carried
  from the old row. The interpreter applies no fill of its own.

### 4.2 Forms

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

### 4.3 Blocks

| Form | Fields and invariants | Meaning |
| --- | --- | --- |
| `Transact(b, o)` | `b` a block term; its type is the block's result, `()` when the block has no `return` | evaluates `b` once against the snapshot to effects `E` and a result; `E` with its derived delete actions is the statement's change set (§6.6); `o` is the block's options |
| `Block(w, k)` | `w : Change`; `k` a block term or the result | `w`'s effects join the statement's; returns `k`'s value |
| `Let(x, e, k)` in a block | `e` a query or a row value | `let! x = e in k`: `e` evaluated once, in the snapshot |
| `LetBang(x, w, k)` | `w` a `Replace`, `Upsert` or `Rejects` | `w`'s effects join the statement's and `x` is bound as §4.4 states |

`TxnOptions`: `retry` is the number of reruns after the first attempt
on a serialization failure; `read_only` and `deferrable` as the block
declares them (a `read_only` block contains no write, and declares no
`durability`); `durability` is `Local` or `Quorum` when declared.

A surface block `transact let x₁ = e₁ … w … return e` is a right-nested
chain of `Let`, `Block` and `LetBang` ending in the result term, inside
one `Transact`. It denotes the same effects and result as the
`let! x₁ = e₁ in … (w₁ ∥ … ∥ wₖ, e)` of §10.3, because every item reads
the snapshot and effects are a set.

A registered transaction is a `Lam` chain over its parameters, as a
registered query is (§1), whose body is one `Transact(b, o)`.

### 4.4 What a `let` binds

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

### 4.5 Snapshot rows

The rows `Delete` and `Replace` take are rows of the relation in the
snapshot: generator binders, `old`, a block `Let` bound to a row read
from the snapshot, and match binders over one. A row bound from a
write is never one of them. An interpreter may still meet a value that
is not in `D(r)` there (a pre-state row deleted twice, say); `Delete`
and `Replace` then derive nothing for it, as the table states.

### 4.6 Delete actions

`Program.retags` maps each relation whose type declares an `on delete
retag D { ... }` to `Lam(x, D { ... })`: the new value of a referrer
`x` that the action retags, its key and header fields `Field(x, f)`,
the payload's literals, and the other own fields of `D` filled as an
insert into `r.D` fills them. Every retag action of one type is one
action, so one term per relation suffices. Which referrers it reaches,
and how `clear` and `cascade` combine with it, is the interpreter's,
by [../spec/semantics.md §6.4](../spec/semantics.md#64-delete-actions).

## 5. Primitives

Arguments are listed in `args` order and evaluated left to right.

| `Prim` | Arguments | `τ` |
| --- | --- | --- |
| `Add`, `Sub`, `Mul`, `Div`, `Rem` | two operands, typed as [../spec/prelude.md §3.1](../spec/prelude.md#31-operators) and [§6.2](../spec/prelude.md#62-arithmetic-on-time) state | the result type |
| `Neg`, `Abs` | one operand | its type |
| `Eq`, `Ne`, `Lt`, `Le`, `Gt`, `Ge` | two operands whose types join, or two decimals of any scales | `Bool` |
| `Concat` | two `Text` or two `Bytes` | the same |
| `CheckedAdd` to `CheckedNeg` | as their operators | `Option` of the operator's result |
| `DecimalDiv` | `a`, `b`, the scale `u : I64` (a constant), a `Rounding` | `Decimal u` |
| `Round` | `a`, the scale `u` (a constant), a `Rounding` | `Decimal u` |
| `Rescale` | `a`, the scale `u` (a constant) | `Decimal u` |
| `Convert { checked }` | the operand; then a `Rounding` from a decimal or float to an integer, the scale and a `Rounding` from a float to a decimal, the scale from an integer to a decimal | the target type, or its `Option` when `checked` |
| `Wrap`, `Unwrap` | one value | the newtype, or its base type |
| `KeyValue` | a key | the key value type (§6) |
| `ParseUuid`, `ToText`, `Print` | one value | `Uuid`, `Text`, `Text` |
| `Parse` | one `Text` | `Option T` |
| `BytesLen` to `Bytes` | as [../spec/prelude.md §4](../spec/prelude.md#4-text) | as there |
| `Micros` to `Years` | one `I64` | `Duration` or `Period` |
| `AddPeriod`, `Zoned` | the operand, a `Period` or a zone name, an `Overflow` | the result, or its `Option` for `Reject` |
| `InZone`, `ToInstant` | as [../spec/prelude.md §6.4](../spec/prelude.md#64-zones) | as there |
| `Span`, `Start`, `End`, `Overlaps`, `Covers` | as [../spec/prelude.md §2.8](../spec/prelude.md#28-span); `Covers`'s second argument is a span or a point, told apart by its type | as there |

## 6. The schema a term refers to

A term names declarations by id; a consumer reads them from the
`Schema` the term was checked against:

- A constructor index is the constructor's position in its sum's
  declaration (`TypeDef::ctors`). A row's fields are the sum's header
  fields, then its constructor's own fields, each in declaration
  order.
- A relation's key fields are `RelationDef::key`. The key value type
  of `r`, which `KeyOf` takes and `KeyValue` returns, is `I64` for one
  key field that is a `Key r` of `r` itself with `generate sequence`,
  and `Uuid` with `generate uuidv7`; the field's type for one other
  key field; and for several, the record of their types, sorted by
  name. `key` of a row whose one key field is a `Key r` of `r` itself
  is that field's value.
- Prelude types are found with `Schema::prelude_type`; their
  constructors are in the prelude's declaration order.

## 7. Code generators

Code generation tools for host languages depend on `adtb-lang` alone.
They compute type, constructor and relation variable ids and call
ordinals as [../spec/wire.md §4.3](../spec/wire.md#43-records-and-sums)
and [§5.1](../spec/wire.md#51-call) assign them, so generated code
agrees with every database built from the same log.

TODO: the API by which `adtb-lang` gives a code generator these ids and ordinals is undecided.
