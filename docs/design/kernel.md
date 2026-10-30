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
| `check_schema(schema)` | `Program`: one arena holding the term of every function, constraint, check, newtype domain, default, computed field, index field, retag delete action (§4.6) and user-defined aggregate of the schema |
| `check_adhoc(schema, program, items, params)` | `Adhoc`: an arena with one term per query, write and `transact` statement, in the order of the items, one checked `Command` per command, and the variable of each ad hoc parameter; grants, revokes and role, peer and temp relation statements are checked and have no term; no bundle is known |
| `check_adhoc_recording(schema, program, items, params, bundles, record)` | as `check_adhoc`, an `explain B::q(args)` checked against `bundles`; with `record`, also appends each schema object the text names, by span, as `schema_refs` reports them, those of items before a refused one included |
| `check_query(schema, program, module, q)` | `Query`: an arena with the term of one registered query |
| `check_txn(schema, program, module, t)` | `Txn`: an arena with the term of one registered transaction |
| `check_feed(schema, module, f)` | the relation variable a feed names and the constructors it streams; refused when the relation is volatile |
| `check_query_in(schema, program, vis, q)`, `check_txn_in(schema, program, vis, t)` | as `check_query` and `check_txn`, for a bundle that reads every module of `vis` |

`program` is the result of `check_schema(schema)`.

Checking diagnostics carry `DiagKind::Invalid` for invalid syntax, types or
acceptance rules, and `DiagKind::Denied` for a program write targeting an
engine-defined relation. Ad hoc, query and transaction checks classify those
writes before normalisation or volatile-read analysis, including writes in
unreached branches and reachable function bodies. `signature::calls` preserves
the category; `signature::bundle` retains its textual error interface.
`analysis::writable_relation(schema, relation)` applies the same target rule to
native bindings such as bulk appenders, returning a denied diagnostic on refusal.
These checks accept client programs; engine-owned command and deployment writers
construct their internal changes separately.

Postconditions on a `Program`:

- `constraints[i]` is the denial query of `schema.constraints[i]`, and
  `checks[i]` that of `schema.checks[i]`: the rows each returns are the
  violations.
- `domains[N]` is the predicate of newtype `N` over its `self`
  variable, at `N`'s base type.
- `aggregates[a]` holds the parts of `schema.aggregates[a]`, `None`
  only when they did not check: `init`, a state; `step`, a `Lam` over
  a state and then an input; `merge`, a `Lam` over two states;
  `finish`, a `Lam` over a state, at the declared result type. A state
  is the `Record` of the declared state fields.
- `index_fields[(r, i, k)]` is the `k`-th field of `r`'s `i`-th index:
  `value`, a `Lam` over a row of `r` in the index's scope, and `path`.
  For an element that is a path through a constructor of a value-level
  sum, `unique (status.Paid.ref)`, `path` lists its steps, each a
  field (`Field(l)`) or a constructor the value the steps before it
  reach must hold (`Ctor(c)`, `c` indexing the sum's declaration order,
  or name order for an anonymous sum); for any other element it is
  empty. Only a row that holds every constructor of every field's
  `path` takes part in the index: it has an entry, it is constrained
  by a `unique`, and only it can conflict in an `Upsert` on one, as the
  existing row or the proposed one. `value` is evaluated only at such a row,
  and is a `Case` per constructor step whose other arms are
  `Unreachable`.

Postconditions on an `Adhoc`:

- `Adhoc.commands` holds one `AdhocCommand` per command, in the
  order of the text. Its `at` is the number of terms of `Adhoc.items`
  the text holds before it, and its `command` holds a term in the
  `Adhoc` arena for each expression of the command, which
  `Command::terms` lists: the statement `explain` explains, as the
  term the same text has as a statement, or the arguments of the
  registered call it explains; the key `pin plan`, `unpin plan`,
  `cancel call` and `terminate session` take, at the key type of
  `catalog::plans`, `sys::calls` or `sys::sessions`; the value of `set
  setting`, a record of the fields of its `catalog::Setting`
  constructor; the source of `export`, a set; the bound of `archive
  audit`, an `I64`; the key of `dump ... encrypt`; and the point of
  `restore ... at`, an `Instant` or a `Position`. Every such term keeps
  the postconditions below, and the acceptance rules a statement's
  term is held to, so it normalises, plans and runs as a statement's
  does. `norm::adhoc` normalises them into `NormalAdhoc.commands`.
- A relation a command names is its `RelId`. `Reindex.index` is the
  position among the relation's indexes, uniques included, of the one
  index `reindex r index (f) in S` names: the one whose fields check to
  the same terms as `f`, up to the names of variables, over the same
  constructor set as `S`. With no `in S` it is the one index with those
  fields, and text matching several is refused naming `in`. Text naming
  no declared index is refused. `None` means every index, the key index
  included.
  `Reindex.online` is `true` unless the text writes `offline`.
- `explain B::q(args)` names a query `bundles` registers for bundle
  `B`; text naming a bundle or a query `bundles` does not hold is
  refused, as are arguments that do not check against the query's
  parameter types, which the caller gives in the types of `schema`.
  The checked form names the bundle and the query, with one term per
  argument.
- When the text declares temp relations, `Adhoc.temps` is the schema
  its terms name declarations of: `schema` with one `RelationDef` per
  temp relation appended to `schema.relations`, in declaration order,
  in a module named `temp`, which no schema module can be named. A
  temp relation of an anonymous record type has a record `TypeDef` of
  its own, named after the relation and appended to `schema.types`.
  `Adhoc::schema(schema)` returns the schema to read; a consumer reads
  every declaration a term names from it. A temp relation has no
  catalog id. Checking the next text of the same interactive
  transaction against `Adhoc::schema(schema)` keeps the earlier
  texts' temp relations in scope.

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

A term the checker returns is not normalised
([../spec/semantics.md §5.11](../spec/semantics.md#511-normalisation)):
calls of schema functions remain `Call` nodes, lambdas and applications
remain, and a `let` remains whatever its bound expression. Totality is
not marked on nodes. The checker normalises every body it checks and
refuses one whose normal form breaks a rule of §7, so the normal form
of every term it returns exists and keeps those rules. §7 states the
normal forms a consumer receives.

### 1.1 Permission requirements

`permission::query`, `permission::txn` and `permission::adhoc` return
relation permission requirements for checked queries, transactions and
ad hoc statements. The ad hoc result has one entry per statement;
commands require their own admission policy.

Preconditions: the checked input and `Program` refer to the supplied
schema and its catalog identities. The effective grants supplied to
`Requirements::missing` belong to the same identity domain and contain
already-applied constructor and field sets, including any history-dependent
freezing or carry. Raw omitted-field grants are not effective snapshots.

Postconditions:

- Requirements use distinct typed catalog identities for relation variables,
  constructors and fields. Missing catalog identities, including those of
  temporary relations, return an error. A row parameter alone introduces no
  relation origin.
- Read requirements retain each value's relation and constructor origins
  through functions, higher-order applications, aggregates, fixpoints,
  nested values and reference following. Ranging reads each selected
  constructor and its declared key fields; storage identity adds no fields.
  A consumed whole row reads every field and constructor of that row's
  origins.
- An aggregate other than `min` and `max`, and a `top`, read whatever
  distinguishes the elements of its set. Rows of one relation in one
  state need only their keys. Each transition and each evaluation of an
  `at` body is its own state.
- Unchanged-field transport in an update adds neither a read requirement
  nor an update requirement for the carried field. A field is unchanged
  only when it is carried from the row the update binds; the same field
  of another row is a change. Explicit use of that field retains its read
  requirement. An identity arm adds no write
  requirement. Same-constructor updates name changed fields; retags name
  the actual source and destination constructor pairs. Insert and delete
  requirements remain separate from retag requirements.
- Upserts include proposed inserts and the user expressions of their
  conflict branches with distinct old and new provenance. Their implicit
  conflict lookup adds no user read requirement. Any observation of a
  stored row's constructor, including an upsert's old row, reads each
  constructor it distinguishes and its declared key fields, as ranging
  does: a `match` its arms' constructors, `tag` the constructors of its
  operand's type, a constructor selection the selected constructors. A
  row built in the query or a parameter adds none. A use of an upsert's
  binding reads only through the branch paths that do not fail. Reject
  destinations add insert requirements. Engine referential actions are
  excluded.
- Coverage returns every missing requirement in deterministic, duplicate-free
  order. Registration mode requires grant option on relation rights;
  file-read capability remains a separate requirement. Coverage uses only
  the supplied effective snapshot and retains no authority between calls.

The provider starts from checked inputs and performs normalization internally.
Checking records access versus synthetic carry purpose; normalization must
preserve that distinction, including when projection eliminates a constructed
row. Ordinary normalized arenas do not independently establish this provenance
contract. Consumers use the requirement provider rather than derive permissions
from executor terms or reader invalidation metadata. Directly imported schema
functions require the separate call and definer policy of their adapter.

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
`Sum` an anonymous sum with constructors sorted by name, `Set` and
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
| `Ctor { ctor, fields }` | `τ` is a declared sum (`ctors` is `Some([ctor])`, or `None` when the sum has one constructor), an anonymous `Sum`, or `Tag T`; `ctor` indexes the declaration order, the name order, or `T`'s constructors; every field of the constructor, header fields included, is given once, sorted by name; a `Tag` constructor has none |
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
| `Agg { op, var, source, body }` | `ty(source)` is a set, `Set⁺` when the range is nonempty (a single generator over a group, with no filter); `τ` is `I64` for `count` and `ty(body)` for `sum`; for `min`, `max` and `avg` it is `ty(body)` exactly when the range is nonempty, else `Option ty(body)`; for `User(a)`, `ty(body)` is the type of the aggregate's one parameter and `τ` its declared result type exactly when the range is nonempty, else `Option` of it |
| `Group { var, source, by }` | `τ = Set { key: ty(by), rows: Set⁺ ty(var) }` |
| `Top { count, keys, var, source }` | `ty(count) = I64`; each key is over `var`, ordered, `true` for `desc`; `τ = ty(source)` |
| `Present { var, source, keys, take, after }` | the final result of an ad hoc statement or a registered query body, nested nowhere else; `take : I64`; `after : Option K` for one key of type `K`, or an `Option` of a record with one field per key, whose fields in name order match the keys in order; `τ = ty(source)`, delivered as a sequence |
| `Fix(binds, b)` | each body has its binder's type; `τ = ty(b)` |
| `At { instant, point, body }` | `ty(point)` is `Instant`, or `Position` when `instant` is false; `τ = ty(body)` |
| `Unreachable` | a position the checker proves no evaluation reaches, by exhaustiveness or by a test that dominates it, at any type; reaching it is a broken invariant |
| writes and blocks | as §4 states |

A `Record` or `Ctor` evaluates its fields in the order the node lists
them, which is name order.

Fields of a record or constructor are evaluated in no stated order: which of several faulting fields is reported is a freedom of [../spec/semantics.md §5.1](../spec/semantics.md#51-scalar-forms), so a consumer may evaluate them in name order.

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
- `Kind::is_write` holds for `Insert`, `Delete`, `Replace`, `Upsert`,
  `Rejects` and `Fail`, and for no other kind. `Terms::writes(t)` holds
  when `t` or a subterm of it is one of these. A term for which
  `Terms::writes` fails produces no effect and never rejects its
  statement with `UserError`.
- Every read inside a write reads the statement's snapshot `D`. No
  write sees another write's effects, except through a variable bound
  by `Let` before a `Block` or by `LetBang` (§4.4).
- A write term is evaluated once per statement unless it sits under a
  `For`, a `Case` arm or an `If` branch. `Fresh(g, r)` draws a new value
  each time it is evaluated, so the interpreter must evaluate the rows
  of an `Insert`, the value bound by a `Let` and the new-value function
  of a `Replace` exactly as often as the term structure says, and no
  more.
- A `Fresh(g, r)` node may be a child of several parents (§2). Each
  path from the root to it is its own occurrence and draws its own
  value on each evaluation: two rows of one `Insert` that share a
  `Fresh` child receive two values. A consumer must not draw once per
  node.
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
| `Replace(r, e, f)` | `e : Set T` of rows of `r`; `f = Lam(x, n)`, `f : T -> T_r`; this `Lam` remains in a normal form (§7) | for each `v ∈ e`: `KeyChanged` if `key_r(f v) ≠ key_r(v)`; nothing if `f v = v` or `v ∉ D(r)`; `rep(r, v, f v)` otherwise | `for x in R do if p then replace r x e° else skip`, with `e` the filtered rows and `f` the completed new value `e°` |
| `Upsert { rel, rows, on, old, new, otherwise, single }` | `rows : Set T`, filled; `on` is `Key` or `Unique(i)`, `i` an index of `rel` with `unique`; `old : T_rel` restricted to the constructor set of `on` (every constructor for `Key`); `new : T`; `otherwise : Change` | for each `n ∈ rows`, `O` = the rows `o` of `rel.S` in `D` whose `on` values equal `n`'s: `ins(rel, n)` if `O` is empty, else the effects of `otherwise` with `old ↦ o`, `new ↦ n`, for each `o ∈ O` | the `let! ξ_V … for new in ξ_V do … if empty(ξ_O) then insert … else for old in ξ_O do w` of §10.3 |
| `Rejects { into, insert }` | `insert` is an `Insert` or `Upsert` whose rows are `For(x, src, body)` with `src` a `Rel(r₀, S)` or a `Prim(ReadFile, [path])`; `into` holds `RejectedRow` | evaluates `body` per record `p` of the source: rows of records that compute are the inner form's rows; a record whose computation reaches `Reject(m)` or does not convert contributes `ins(into, { load: ℓ, line, source, raw, reason: m })` instead; `ℓ` is one fresh value per statement | `insert r (for x in e₀ do e) rejects into j` (§6.7) |
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
| `ReadFile { format, pins }` | the path, a `Text` | `Set T`, `T` the declared row type; constructor names in the files are read at schema version `pins`, or at the caller's when `None` ([../spec/interop.md §3](../spec/interop.md#3-file-scans)) |

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
  is that field's value. For a `catalog::` or `sys::` relation whose
  one key field is a `Key r` of `r` itself, with no generator, it is
  `I64`.
- Prelude types are found with `Schema::prelude_type`; their
  constructors are in the prelude's declaration order.
- `Schema::self_key(r, f)` holds when key field `f` of `r` is a `Key r`
  of `r` itself.
- `Schema::reference_fields(r)` returns every field of `r`'s sum whose
  type is `Key t` or `Option (Key t)`, except a key field of `r` whose
  target `t` is `r`, header fields first, then each constructor's own in
  declaration order. Each names its holding constructors, its position
  in their rows, its target and constructor selection, whether it is an
  `Option`, its delete action, and the index that leads with it over
  every holding constructor: the key when it is `r`'s first key field,
  else the first declared key, unique or index of `r` that does, else
  its implied index. `Schema::implied_indexes(r)` returns exactly the
  fields whose index is the implied one, and `Schema::ids` holds the id
  of the index each names: `keys`, `indexes` or `implied`, for every
  relation of a user module. A consumer decides which fields of a row
  are references only by `reference_fields`.
- `Schema::ids` holds the catalog id of each object
  ([../spec/catalog.md §1.2](../spec/catalog.md#12-identity-of-catalog-objects)):
  every object of a user module has one, and the engine's types,
  constructors, relation variables, fields and indexes have their
  fixed ids
  ([../spec/catalog.md §4.6](../spec/catalog.md#46-constructor-ids-of-engine-defined-sums)),
  each relation's key index in `CatalogIds::keys` and its uniques in
  `CatalogIds::indexes`.
  A stored view whose element type is anonymous has its row type, its
  constructor and its fields' ids in `view_types`, `view_ctors` and
  `view_fields`, by the view's function; a view of a declared type has
  none there. A schema `schema::build`
  returns numbers its user objects as one migration declaring its
  files' declarations in order would; one `Source::schema` returns
  carries the ids of its log ([schema-change.md §4](schema-change.md#4-the-schema-transition)).
- `Schema::stage(r, c)` is the write state of constructor `c` in
  relation `r`, and `Schema::value_stage(t, c)` that of `c` as a value
  of `t` stored in a field; both are `Enabled` in a schema
  `schema::build` returns.

## 7. Normal forms

`adtb-lang` normalises every body it checks. A consumer receives normal
forms and does not normalise them again.

```text
fn norm::program(schema: &Schema, p: &Program) -> Result<NormalProgram, Error>
fn norm::adhoc(schema: &Schema, p: &Program, a: &Adhoc) -> Result<NormalAdhoc, Error>
fn norm::query(schema: &Schema, p: &Program, q: &Query) -> Result<NormalQuery, Error>
fn norm::txn(schema: &Schema, p: &Program, t: &Txn) -> Result<NormalQuery, Error>
```

Preconditions: `p` is the result of `check_schema(schema)`; `a`, `q` and
`t` are the results of `check_adhoc`, `check_query` and `check_txn`
against `schema` and `p`.

Each result is a new arena. `NormalProgram` holds the normal form of
every constraint, check, newtype domain, default, computed field, index
field, retag value and user-defined aggregate part of the schema, and of every function whose
parameters and result are first-order, over that function's parameter
variables. A function that takes or returns a function has no normal
form of its own, and neither has a Wasm or host function. A computed
field is a body over its `self` variable; an index field's `value` and
a retag value are each a body over the row's variable, and an index
field keeps its `path`. An aggregate's `step`, `merge` and `finish`
are each a body over their variables, `None` for the aggregate when
one of its parts is not first-order. `NormalQuery` is a
registered query's or transaction's body over its parameter variables;
one with no parameters has one variable of type `()`. `NormalAdhoc`
holds the normal form of each item and, in `commands`, each command of
`Adhoc.commands` at the same place, its terms replaced by their normal
forms. `Error` is never returned for a body the checker accepted.

Postconditions on a normal form, beyond those of §1:

- It contains no `Lam` and no `App`, except that the third operand of
  a `Replace` is a `Lam` over the old row (§4.2) whose body is a
  normal form.
- A `Call` remains only of a Wasm or host function. Every schema
  function is inlined, a generic one at the type arguments its call
  solved, and no type in the arena mentions a type parameter of an
  inlined function.
- Normalisation never duplicates or drops a term that contains
  `Fresh`. A `Let` whose bound expression contains `Fresh` is kept, and
  its variable is substituted nowhere.
- A `Let` remains only where its bound expression is partial or
  contains `Fresh`, and no rewrite of
  [../spec/semantics.md §5.11](../spec/semantics.md#511-normalisation)
  applies anywhere in it.
- Every node has the type of the node it came from, or a subtype of it.
- A node shared by several parents in the checked term is one node in
  the normal form wherever its free variables have the same values.
- Every `fix` is stratified
  ([../spec/semantics.md §5.10](../spec/semantics.md#510-fixpoints)).
- Every `sum` and `avg` ranges over bindings, never over values a
  `select` may have merged
  ([../spec/queries.md §10.1](../spec/queries.md#101-aggregates-range-over-bindings)).

### 7.1 Totality

```text
fn Totality::new(schema: &Schema, hiding: bool) -> Totality
fn Totality::total(&mut self, terms: &Terms, t: Tm) -> bool
fn Totality::draws(&mut self, terms: &Terms, t: Tm) -> bool
fn Totality::copyable(&mut self, terms: &Terms, t: Tm) -> bool
```

`total` returns whether `t` cannot abort, by
[../spec/semantics.md §3.4](../spec/semantics.md#34-totality). `hiding`
is set for a bundle declaring `hide new constructors`; every `Deref` is
then partial.

Precondition: every term one `Totality` sees belongs to one arena, and
no term changes after the first question about it.

Postcondition: a term classified total never faults, never fails and
terminates. A consumer may evaluate it early, more than once or not at
all, and may move it past a guard, unless it contains `Fresh`, which
is evaluated as often as §4.1 states. A partial term keeps its order
relative to other partial terms and stays under every guard that
dominated it.

`draws` returns whether `t` contains `Fresh` or a call of a host
function not declared deterministic. `copyable` returns whether `t` is
total and does not draw.

Partial, besides the primitive faults of
[../spec/prelude.md](../spec/prelude.md): integer and decimal `sum`, a
user-defined aggregate, `top` whose count is not a nonnegative literal,
an unbounded `fix`, a `Deref` of a key that is not a reference field's
value, a Wasm or host `Call`, `At`, `Fail`, `Reject` and `Replace`.
`Try` is total whatever its operand. A `fix` is bounded when no
constructing operation over a value of its own stratum can produce an
element of one of its relations; an operation whose result is used only
in a condition, a comparison or an aggregate key does not count.

## 8. Code generators

Code generation tools for host languages depend on `adtb-lang` alone.
They compute type, constructor and relation variable ids and call
ordinals as [../spec/wire.md §4.3](../spec/wire.md#43-records-and-sums)
and [§5.1](../spec/wire.md#51-call) assign them, so generated code
agrees with every database built from the same log.

`adtb-lang` gives a generator these ids and ordinals through
`signature`:

```text
replay(log) -> Result<Source, ReplayError>
bundle(source, text) -> Result<Bundle, String>
Bundle { header: BundleHeader, schema: Schema, calls: Vec<Signature> }
Signature { ordinal: u32, name: String, kind: CallKind, params: Vec<(String, Ty)>, result: Ty }
CallKind = Query | Txn | Fn
ReplayError { migration: u64, message: String }
```

Preconditions: `source` is `replay` of the log up to the version the
bundle file `text` pins.

Postconditions:

- `replay(log)` is the source every database that applies `log` in
  order holds; it fails at the first migration that does not apply,
  numbered from 1.
- `Source::schema()` of it holds in `Schema::ids` the catalog id of
  every type, constructor and relation variable that such a database
  assigns.
- `bundle` returns the callables of `text` in ordinal order, each with
  the ordinal `adtb` assigns at registration: each `import fn` in
  import order, then each query and transaction in item order, from 1.
  A bundle function takes none. `params` are the declared parameters
  in order; `result` is the declared result type.
- `schema` is that of `source` with the bundle's functions added to
  its first imported module, as registration checks the bundle.
- `bundle` fails as registration refuses the file: a parse error, a
  module import of no user module, a callable that does not check or
  is declared twice, a generic imported function, a feed or an
  aggregate.

`adtb-codegen` is the generator for Rust. Its library takes the log
and bundle files and returns Rust source, for a `build.rs`:

```text
generate(log: &[&str], bundles: &[BundleSource]) -> Result<String, Error>
BundleSource { module: &str, text: &str }
read_log(dir) -> io::Result<Vec<String>>
```

The source holds module `schema`, with `LOG` and the host types of
the schema at the end of the log, and one module per bundle, with
`BUNDLE`, `Bundle`, `Error` and one value per callable. The host types
and calls it emits are those of
[server.md §4.5](server.md#45-typed-bundles).

## 9. Textual syntax

`adtb-lang` reads and prints the textual syntax of
[../spec/interop.md §1](../spec/interop.md#1-textual-value-syntax)
(`textual`) over any consumer's values. A consumer implements
`Textual` for its value type; `adtb-reference` does for its `Value`
and `adtb` for its executor's `Val`.

```text
Form::of(schema, ty) -> Form
Form::has_syntax(&self) -> bool
print<V: Textual>(v, form) -> Result<String, PrintError>
to_text<V: Textual>(v, form) -> Result<String, PrintError>
parse<V: Textual>(text, form) -> Result<V, ParseError>
read_document<V: Textual>(schema, text) -> Result<Document<V>, ParseError>
print_document<V: Textual>(schema, doc) -> Result<String, PrintError>
Document<V> { header: Option<Header>, relation: RelId, rows: BTreeSet<V> }
Header { edition: String, version: u64, hash: String }
ParseError { offset: usize, message: String }
PrintError { message: String }
```

A `Form` is the shape of the values of a type as the syntax needs it,
computed once from the schema: constructor and field names, key
forms, the newtypes and prelude types with forms of their own (`Uuid`,
`Span`), and `Opaque` for a type parameter, function, group or
`Change`. It holds no schema reference, so a consumer may keep it in a
compiled plan. `has_syntax` is false when an `Opaque` form occurs in
it.

`Textual` gives the printer a value's parts and the reader a way to
build one:

- `scalar` is a value of a base type as a `Scalar`: every integer an
  `Int`, a decimal its unscaled digits, `Bytes`, `SortKey` and `Uuid`
  as `Bytes`, a date in days since 1970-01-01, a time of day in
  microseconds since midnight, an `Instant` in microseconds since
  1970-01-01T00:00:00Z, and a civil date-time in microseconds since
  1970-01-01T00:00:00. `of_scalar(s, b)`
  builds the value of base type `b` that `s` holds.
- `ctor` and `fields` are a constructor value's index and fields, its
  header fields first, each in declaration order; `fields` of a
  record or a composite key value gives them sorted by name.
  `of_ctor` and `of_record` take the fields in that order, each with
  its name; a record with none is the unit value.
- `key` is the key value of a key and `of_key` the key of a key value,
  whose type is the key value type of
  [§6](#6-the-schema-a-term-refers-to).
- `elems` is a set's elements, ascending, or an ordered result's
  elements in its order; `of_set` takes elements in any order, with
  repeats.
- `zone_exists(name)` says whether the consumer's zone rules name the
  zone.
- `Ord` is the model's order on ordered types
  ([../spec/model.md §10](../spec/model.md#10-order)); the reader
  compares a span's ends with it.

Preconditions: `ty` is a type of `schema`. Each accessor returns
`None` at a value of another shape: `scalar` at anything but a
scalar, `ctor` at anything but a constructor value, `fields` at
anything but a constructor value or a record, `key` at anything but a
key when the consumer's keys are values of their own, `elems` at
anything but a set or an ordered result.

Postconditions:

- `print` and `to_text` fail with a `PrintError` when `v` is not a
  value of the type `form` was computed from, or `form` has `Opaque`
  on the path of `v`; they never panic. A consumer reports the
  failure as a broken checker guarantee.
- `parse(&print(&v, &f)?, &f) == Ok(v)` for every value `v` of form
  `f`, and equal values print identically. Two consumers' values that
  are the same model value print identically.
- `parse` returns `Ok(v)` only for a value of the form; other text
  gives a `ParseError` holding the byte offset where reading stopped.
  At `Opaque` it fails with "this type has no textual syntax".
- `Uuid` has every `Bytes` representation in memory. `print` uses
  `uuid("8-4-4-4-12")` for sixteen bytes and `Uuid(0x...)` for every
  other length, without padding or truncation. `parse` accepts both
  wrappers at every byte length the wrapper represents; it does not
  check the newtype's write-time domain. A scalar of another kind
  remains a shape error.
- `to_text` is the `ToText` primitive: a number or time value as its
  literal, a sixteen-byte `Uuid` in its lowercase 8-4-4-4-12 form,
  any other `Uuid` as `print` gives it, a newtype over `Text` as its
  text; any other value as `print` gives it.
- `read_document` of `print_document(doc)` is `Ok(doc)`. A document's
  header is returned as written; translating a document written at
  another version is the caller's.
