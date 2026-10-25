# Semantics

This part defines what programs mean. It gives a small kernel calculus,
its types in brief, and a set-theoretic denotation of every kernel
form: queries as functions from a database state to a set, writes as
change sets, statements as one multiple assignment, constraints as
queries that must return nothing, serializability over constructor
relations, and schema operators as maps between database states of two
versions. Every surface construct of [syntax.md](syntax.md),
[queries.md](queries.md), [writes.md](writes.md) and
[constraints.md](constraints.md) is defined by its desugaring into the
kernel (§10). Where parts disagree on the meaning of a program, the
conformance suite wins, then this part, then the surface parts; the
edition that follows corrects the losing text. Full surface typing is in [typing.md](typing.md).

## 1. Conventions

### 1.1 Metanotation

```text
∅, {a, b}, A ∪ B, A ∖ B, A ⊎ B      finite sets; ⊎ is union of disjoint sets
⦃a, a, b⦄                           finite multisets
f[x ↦ v]                            f updated at x
ρ                                   an environment: variables to values
σ                                   a statement context (§4.3)
⟦e⟧ ρ σ                             the denotation of e; σ is omitted where unused
O(X) = X ⊎ { ↯f | f ∈ Fault } ⊎ { ⊗x | x ∈ RejectReason }
                                    outcomes: a value, a fault or a rejection
```

`↯f` is the outcome of a computation that faulted with `f`, a
constructor of the prelude sum `Fault`
([prelude.md §2.5](prelude.md#25-fault)). `⊗x` is the outcome of a
statement rejected for reason `x`, a constructor of the prelude sum
`RejectReason` ([catalog.md §4.2](catalog.md#42-reasons)): `fail`, a key
violation, a constraint violation, a statement exceeding its limits
(`StatementLimit` and the other limit reasons of
[catalog.md §8.3](catalog.md#83-limits-and-deadlines)). Both are aborts; `ω` ranges over
them. Only `try` tells them apart: it catches faults and never
rejections, limits included. Limits are not part of any equation: any
statement may be rejected for a limit at any step. Outcomes compose strictly:

```text
v ⇐ o ; k(v)  =  k(v)   if o = v ∈ X
              =  ω      if o = ω, an abort
```

The operands of one expression, within one binding, are sequenced left
to right; the members of a family indexed by the bindings of a set, or
by the fields of one record or constructor value, are gathered:

```text
seq (o₁, …, oₙ)       =  (v₁, …, vₙ)    if every o_i = v_i
                      =  o_j            for the least j with o_j an abort, otherwise
gather (o_i)_{i ∈ I}  =  (v_i)_{i ∈ I}  if every o_i = v_i
                      =  o_j            for some j with o_j an abort, otherwise
```

Within a binding the first abort reached is the outcome. When several
bindings abort, the outcome is any one of their aborts, so a `try` over
several faulting bindings returns `Err` of the fault of any one of them.
Likewise when several fields of one record or constructor value abort.

### 1.2 Kernel and surface

The kernel (§2) is what the semantics is given on. Every surface form
is a rewrite into it (§10). The rewrite is total on well-typed surface
programs and preserves typing: a surface program of type `T` desugars to
a kernel term of type `T`.

### 1.3 Conformance

Every equation in this part reads as a program over finite sets. An
implementation conforms
when, for every program and every state, its outcome is one the
equations allow. The equations fix the outcome except in six places,
each stated where it arises:

| Freedom | Section | Conformance compares |
| --- | --- | --- |
| Which abort is reported when several bindings abort, and which fault a `try` over them returns | §1.1 | that both outcomes are aborts; under `try`, that the fault is one some binding raises |
| Which abort is reported when several fields of one record or constructor value abort | §5.1 | that both outcomes are aborts; under `try`, that the fault is one some field raises |
| The values of generated keys and generated clock fields | §4.3 | results up to a bijection on generated values |
| The fold order of an aggregate whose operator is not associative and commutative | §5.7 | results within a bound that grows with input size |
| The step at which a statement exceeds its limits, an unbounded fixpoint included | §5.10 | that both are rejected for a limit |
| Which abort is the outcome when several `let!` bindings, writes or the result of a block abort | §6.6 | that both outcomes are aborts |

The bound for a float aggregate: two results of an aggregate over `n`
values `x_i` agree when they differ by at most `n * eps * sum(|x_i|)`,
with `eps` the machine epsilon of the float type. Every other value is
compared exactly.

Rulings: project/semantics/denotational, project/semantics/kernel/core, project/semantics/covers/queries, project/semantics/covers/writes, project/semantics/covers/constraints, project/semantics/covers/isolation, project/semantics/covers/evolution, project/oracle/float/tolerance, spec/tiers/three-tiers, spec/field-faults/any

## 2. Kernel syntax

### 2.1 Schemas

A schema `Σ` is the fold of the operator log up to a version (§9.5). It
supplies:

```text
types(Σ)        declared sums N, each with type parameters, a header
                (fields common to every constructor), constructors
                ctors(N) and, for each C, its fields
rels(Σ)         relation variables r, each with element type T_r (a sum),
                key fields key_r (declared in T_r, of type Key r),
                declared uniques, named sets set S of r = C₁ | … | Cₖ,
                and fill clauses: for each field, an optional default
                expression or generator g (sequence, uuidv7, clock)
refs(Σ)         reference fields: a field f of a constructor C of T_r'
                of type Key r.S or Option (Key r.S), with a delete action
                restrict | cascade | retag D { … } | clear
                (clear only on Option (Key r.S))
cons(Σ)         named constraints, each a closed kernel query (§7): the
                constraints of Σ, the domains of its newtypes (§7.2),
                and every dropped constraint, unique or check, and every
                key replaced by a relaxing rekey, whose drop is still
                pending
fns(Σ)          schema functions, each a closed kernel term (§5.11)
stage(Σ)        for each constructor, of a relation-level or a
                value-level sum, its lifecycle stage
```

A pending drop is a constraint of `cons(Σ)` like any other: every
later operator carries it and rechecks it (§9.2), and an operator that
leaves one without an image is refused.

A constructor set `S` of `r` is a nonempty subset of `ctors(T_r)`,
written by listing or by a declared name; a declared name is replaced
by its list before the kernel sees it. `r` alone stands for `r.ctors(T_r)`.

### 2.2 Types

```text
τ ::= B                       base type (model.md §3)
    | N τ₁ … τₙ               declared sum, record or newtype, applied
    | N τ̄ . S                 row type of constructor set S ⊆ ctors(N)
    | { l₁: τ₁, …, lₙ: τₙ }    anonymous record
    | Set τ | Set⁺ τ          finite set; nonempty finite set
    | Key r.S                 reference into constructor set S of r
    | τ → τ                   function
    | Change                  change set
```

A surface group binder of type `Group K T` is a kernel record
`{ key: K, rows: Set⁺ B }`, `B` the record of the grouped bindings;
used as a set it is the nonempty set of values of `T` the rows give
(§10.1).

`N τ̄ . ctors(N)` is `N τ̄`. A function may take and return functions.
No function type occurs in the element type of a relation, a field, a
set element, a type argument, a query result or a parameter of a
registered query; normalisation removes every function (§5.11). A result sum anonymous to a query is structural: two with the
same constructors and fields, in any order, are one type
([typing.md](typing.md)). `Bool` is the prelude
sum `False | True`; `Option` and `Result` are prelude sums
([model.md §5.2](model.md#52-library-sums)). `Set⁺ τ` is a kernel type
with no surface spelling; it types the rows of a group (§5.8). The
surface unit type `()` is the kernel record type `{}`.

### 2.3 Expressions

```text
e ::= x                               variable
    | c                               literal of a base type
    | op(e₁, …, eₙ)                   primitive operation (model.md)
    | { l₁: e₁, …, lₙ: eₙ }            record
    | e.l                             field
    | C { l₁: e₁, …, lₙ: eₙ }          constructor value, header and own fields
    | case e of { C₁ x₁ → e₁ ; … ; Cₙ xₙ → eₙ }
    | if e then e else e
    | let x = e in e
    | λ(x : τ). e  |  e e
    | key e  |  tag e
    | keyof r e                       the key value of r with key fields e
    | deref r.S e                     the row a reference names
    | try e
    | r.S                             constructor-set relation
    | inserted r.S | deleted r.S | prior r.S    (constraint bodies only)
    | e ▷ S                           restriction of a set to constructors S
    | {}  |  {e}  |  e ∪ e
    | for x in e do e
    | empty e  |  exists e
    | agg α (x in e . e)
    | group (x in e) by e
    | top e by (e dir, …, e dir) (x in e)    dir ::= asc | desc
    | fix X₁ = e₁, …, Xₙ = eₙ in e      n ≥ 1, simultaneous
    | fresh g                         generated value, write positions only
    | only e                          the element of a one-element set
```

A boolean condition is an expression of type `Bool`. `and`, `or` and
`not` are not kernel forms (§10.1). `fail` is a write term only (§2.4).

### 2.4 Writes and statements

```text
w ::= skip
    | insert r e                      e : Set (T_r)
    | delete r e                      e : Set (T_r), rows of the snapshot
    | replace r e e                   old row, new whole value
    | w ∥ w
    | for x in e do w
    | if e then w else w
    | case e of { C₁ x₁ → w₁ ; … ; Cₙ xₙ → wₙ }
    | let x = e in w
    | fail e
    | insert r (for x in e do e) rejects into j    e a relation or external source

b ::= let! x = e in b                 block binding, evaluated once
    | (w, e)                          the block's writes and its result

s ::= transact b                      a statement
```

A write term is an expression of type `Change`; the grammar is kept
apart because write terms may appear only in the positions above
([writes.md §1.1](writes.md#11-queries-are-pure)).

### 2.5 Side conditions

A kernel term is well formed when, beyond typing (§3):

- every `fix X₁ = e₁, …, Xₙ = eₙ in e` is stratified: each `X_j`
  occurs in every `e_i` only at positive positions (§5.10);
- no λ is a free variable of a `fix` body other than through a term
  that normalisation (§5.11) removes;
- no schema function refers to itself, directly or through other
  functions; recursion is written only with `fix`;
- `fresh g` occurs only in a fill `fill_r` (§10.3): of the rows of an
  `insert`, of a `let!` binding whose rows an `insert` of the same
  block consumes, of the new value of a `replace`, or of the new value
  a delete action's `retag` derives (§6.4); or as the `load` value of an
  `insert … rejects into` (§6.7); never under a λ;
- `only e` occurs only in the binding of a single-row upsert (§10.3),
  where `e` has exactly one element;
- `inserted`, `deleted` and `prior` occur only in the body of a
  constraint;
- a `case` names each constructor of its scrutinee's constructor set
  exactly once.

Rulings: project/semantics/kernel/core, query/ctor-relations, lang/family/comprehension, lang/recursion/fix, lang/abstraction/higher-order, lang/abstraction/ho-recursion/first-order-recursion, write/primitive/delta, write/surface/txdata, derived/transition/change-visible, types/fault-forms/try, lang/group/into, lang/nested-order/top-n, lang/group/ctor-key/tag-type, model/typed-key, model/user-params, model/prelude-bool, spec/lambda/params/curried, spec/writes/rejects-source/one-source, spec/tiers/three-tiers

## 3. Typing

### 3.1 Judgements

```text
Γ ⊢ e : τ          expression e has type τ under Γ
Γ ⊢ w : Change     w is a write term
Γ ⊢ s : τ          statement s returns τ
Γ ⊢ e total        e cannot abort (§3.4)
```

`Γ` maps variables to types and holds the schema `Σ`. Surface typing,
inference and generics are in [typing.md](typing.md); this section
gives the rules for the kernel forms whose typing the semantics relies
on. Subtyping is that of [typing.md §3](typing.md#3-subtyping), with one
kernel addition: `Set⁺ τ <: Set τ`.

### 3.2 Rules

```text
r ∈ rels(Σ)   S ⊆ ctors(T_r)
──────────────────────────────────────── (rel)
Γ ⊢ r.S : Set (T_r . S)

Γ ⊢ e : Set (N . S')   S ⊆ S'
──────────────────────────────────────── (restrict)
Γ ⊢ e ▷ S : Set (N . S)

Γ ⊢ e : N . S    for each C ∈ S:  Γ, x_C : N . {C} ⊢ e_C : τ
──────────────────────────────────────── (case)
Γ ⊢ case e of { C x_C → e_C }_{C ∈ S} : τ

Γ ⊢ e₁ : Set τ₁    Γ, x : τ₁ ⊢ e₂ : Set τ₂
──────────────────────────────────────── (for)
Γ ⊢ for x in e₁ do e₂ : Set τ₂

Γ ⊢ e : Set τ
──────────────────────────────────────── (empty, exists)
Γ ⊢ empty e : Bool      Γ ⊢ exists e : Bool

Γ ⊢ e₁ : Set τ   Γ, x : τ ⊢ e₂ : τ_α   α a monoid aggregate
──────────────────────────────────────── (agg-monoid)
Γ ⊢ agg α (x in e₁ . e₂) : ρ_α

Γ ⊢ e₁ : Set τ   Γ, x : τ ⊢ e₂ : τ_α   α a semigroup aggregate
──────────────────────────────────────── (agg-semigroup)
Γ ⊢ agg α (x in e₁ . e₂) : Option ρ_α

Γ ⊢ e₁ : Set⁺ τ   Γ, x : τ ⊢ e₂ : τ_α   α a semigroup aggregate
──────────────────────────────────────── (agg-nonempty)
Γ ⊢ agg α (x in e₁ . e₂) : ρ_α

Γ ⊢ e₁ : Set τ   Γ, x : τ ⊢ e₂ : κ
──────────────────────────────────────── (group)
Γ ⊢ group (x in e₁) by e₂ : Set { key: κ, rows: Set⁺ τ }

Γ ⊢ n : I64   Γ ⊢ e₁ : Set τ   Γ, x : τ ⊢ k_i : κ_i, each κ_i ordered
(k₁, …, k_j) injective on e₁
──────────────────────────────────────── (top)
Γ ⊢ top n by (k₁ d₁, …, k_j d_j) (x in e₁) : Set τ

Γ' = Γ, X₁ : Set τ₁, …, Xₙ : Set τₙ   Γ' ⊢ e_i : Set τ_i for each i   Γ' ⊢ e : τ'
every X_j positive in every e_i
──────────────────────────────────────── (fix)
Γ ⊢ fix X₁ = e₁, …, Xₙ = eₙ in e : τ'

Γ ⊢ e : Key r.S
──────────────────────────────────────── (deref)
Γ ⊢ deref r.S e : T_r . S

Γ ⊢ e : T_r . S   (the relation r of e is statically known)
──────────────────────────────────────── (key)
Γ ⊢ key e : Key r.S

Γ ⊢ e : { f₁: τ₁, …, f_j: τ_j }   f₁ … f_j the key fields of r
──────────────────────────────────────── (keyof)
Γ ⊢ keyof r e : Key r

Γ ⊢ e : N τ̄ . S
──────────────────────────────────────── (tag)
Γ ⊢ tag e : Tag N

Γ ⊢ e : τ
──────────────────────────────────────── (try)
Γ ⊢ try e : Result τ Fault

Γ ⊢ e : Set τ
──────────────────────────────────────── (only)
Γ ⊢ only e : τ
```

`(k₁, …, k_j) injective on e₁` holds when the list is a key of the elements of `e₁`
as [queries.md](queries.md#143-top-n) states; otherwise `top` is refused.
In (fix), the `τ_i` are the least types under
[typing.md §3](typing.md#3-subtyping) at which every `e_i` has type
`Set τ_i`; a `fix` with no such types is a type error.
`τ_α` and `ρ_α` are the argument and result types of aggregate `α`
(§5.7). Element types of `Set` are first-order: no function type occurs
in them.

Write rules:

```text
Γ ⊢ e : Set (T_r . S)                     ⟹  Γ ⊢ insert r e : Change
Γ ⊢ e : Set (T_r . S)                     ⟹  Γ ⊢ delete r e : Change
Γ ⊢ e₁ : T_r . S   Γ ⊢ e₂ : T_r . S'      ⟹  Γ ⊢ replace r e₁ e₂ : Change
Γ ⊢ w₁ : Change   Γ ⊢ w₂ : Change         ⟹  Γ ⊢ w₁ ∥ w₂ : Change
Γ ⊢ e : Set τ   Γ, x : τ ⊢ w : Change     ⟹  Γ ⊢ for x in e do w : Change
Γ ⊢ e : Text                              ⟹  Γ ⊢ fail e : Change
Γ ⊢ e_w : Change   Γ ⊢ e_r : τ            ⟹  Γ ⊢ transact (e_w, e_r) : τ
```

`replace r e₁ e₂` with `S ≠ S'` is a retag.

The relation of a row is statically known for a binder over a relation
variable, constructor relation, constructor set or backlink of `r`; for
a binder over a key-preserving query of `r` (one whose element is the
binder of such a generator itself, possibly through `top` or a
restriction); for a binder over `inserted`, `deleted` or `prior` of
`r`; for a binder over a view returning rows of one relation
`r`; for a row bound by `let` or `let!` from a write into `r`; and for a
value of a type with a field of type `Key r` that names its own row.
`key e` is refused for any other `e`.

### 3.3 Exhaustiveness

A kernel `case` names every constructor of its scrutinee's set exactly
once and has no guards and no wildcard. Surface exhaustiveness
([typing.md §9.2](typing.md#92-exhaustiveness)) is what makes the
pattern compilation of §10.2 produce such a `case`.

### 3.4 Totality

`e` is total when it contains none of:

- a partial primitive ([typing.md §10.1](typing.md#101-total-and-partial-expressions),
  [prelude.md](prelude.md)), `add_period` with either overflow rule
  included;
- an aggregate whose combining operator or final map is partial on its
  type (`sum` over an integer type);
- `top` whose count is not a nonnegative literal;
- a `fix` that is not bounded (§5.10), which may not terminate;
- a `deref` of a key that is not a reference field's value;
- any `deref`, and every path through a reference, in a bundle
  declaring `hide new constructors` (§9.6);
- an `at` block ([derived.md §6.2](derived.md#62-naming-a-past-state)),
  which is rejected with `HistoryUnavailable` when its position is not
  retained;
- an application of a function whose body is not total.

`try e` is total whatever `e` is. A total expression terminates and
denotes a value in every environment that its type admits; it never
faults. Limits (§1.1) are outside totality.

Rulings: lang/binder/ctor-row, lang/patterns/or-binder/ctor-set, model/ref-subtyping/ctor-set-subsumption, query/compile-time, lang/wildcard/scalars-only, lang/empty-agg/nonempty, types/join/lub-exact-fields, types/sub-records/exact, types/eq-types/all-first-order, lang/collection/set-agg/over-binders, lang/recursion/negation/stratified, types/fault-forms/checked, spec/fix/typing/least, spec/totality/add-period/partial, spec/totality/at-block/partial

## 4. Values and states

### 4.1 Values

The value domain `V` is the least set containing:

```text
base values             as model.md §3 defines each base type
C⟨h̄, ḡ⟩                 a constructor value: constructor C, header fields h̄,
                        own fields ḡ; records are one-constructor values
{ l₁: v₁, …, lₙ: vₙ }    anonymous records
finite sets of values   the values of Set τ and Set⁺ τ
key values              the values of Key r.S, equal to the key fields' value
```

and, for terms with function types only, the functions `V → O(V)`.
`==` on `V` is the equality of [model.md §9](model.md#9-equality); it is
identity on canonical values, so two values are equal exactly when they
are indistinguishable. Sets are taken modulo `==`. `tag(C⟨…⟩) = C`.

A row of `r` is a constructor value of `T_r`. `key_r(v)` is the value
of its key fields. A row's key and header fields belong to the row
whatever its constructor.

### 4.2 Database states

A state `D` over `Σ` assigns to every constructor relation `r.C` a
finite set `D(r.C)` of values with `tag v = C`. The catalog relations
are relation variables like any other
([catalog.md](catalog.md#1-system-relations)). Sums are closed: `r` has no
rows other than those of its constructor relations.

```text
⟦r.S⟧ D  =  ⊎_{C ∈ S} D(r.C)
```

The union is disjoint because each row carries its constructor. `⟦r⟧ D`
is the union over every constructor of `T_r`. A state is valid when
every constraint of `Σ` holds in it (§7). The valid states of `Σ` are
`DB(Σ)`.

Heaps, pages and index structures are representations of `D`; no
program observes them.

### 4.3 Statement context

Every statement is evaluated in a context

```text
σ = (D, Σ, v, γ)
```

where `D` is the statement's snapshot, `Σ` the schema at version `v`,
and `γ` a supply of generated values. `γ` assigns to each evaluation
instance of a `fresh g` (its occurrence together with the values of the
variables bound around it) a value of the generator `g`, such that:

- for a key generator of `r`, the values are pairwise distinct,
  distinct from every key any committed row of `r` has held (including
  rows the statement's version does not see, §9.6), and distinct from
  every value already issued
  ([writes.md §2.3](writes.md#23-defaults-and-generated-keys));
- for the clock generator, the value is an `Instant`, the wall-clock
  time at which the statement applies, and every clock instance of one
  statement has the same value
  ([writes.md §2.3](writes.md#23-defaults-and-generated-keys)).

Generators and defaults are clauses of a relation variable, not of a
type. Constructing a value is pure; only a write into a relation
variable draws from `γ`, through the fills its desugaring adds to an
insert, to the new value of an update or retag, and to a delete
action's retag (§6.4, §10.3).

The denotation of a statement is a function of `σ`. Two conforming
implementations may choose different `γ`; their outcomes agree up to the
renaming of generated values. Queries read no clock: the current time
is a parameter or a generated field.

Rulings: model/sets, model/nominal, model/one-ctor-sum, model/record-is-ctor, sum/closed, sum/disjoint-total, sum/relation, model/one-key, model/declared-key, model/no-collections, model/sum-header, model/set-of-type, lang/results/nested, types/eq/leibniz, engine/guarantees-only, engine/catalog-relations, types/now/param-only, types/id-type/key-type, types/keygen/promise/distinct, spec/tiers/three-tiers

## 5. Queries

`⟦e⟧ ρ σ ∈ O(V)`. `D` below is the snapshot of `σ`.

### 5.1 Scalar forms

```text
⟦x⟧ ρ                    = ρ(x)
⟦c⟧ ρ                    = c
⟦op(e₁,…,eₙ)⟧ ρ          = (v₁,…,vₙ) ⇐ seq(⟦e_i⟧ ρ) ; op(v₁,…,vₙ)
⟦{ l_i: e_i }⟧ ρ         = (v_i) ⇐ gather(⟦e_i⟧ ρ)_i ; { l_i: v_i }
⟦C { l_i: e_i }⟧ ρ       = (v_i) ⇐ gather(⟦e_i⟧ ρ)_i ; C⟨v_i⟩
⟦e.l⟧ ρ                  = v ⇐ ⟦e⟧ ρ ; field l of v
⟦key e⟧ ρ                = v ⇐ ⟦e⟧ ρ ; key_r(v)
⟦keyof r e⟧ ρ            = v ⇐ ⟦e⟧ ρ ; the key value of r whose key fields are v
⟦tag e⟧ ρ                = v ⇐ ⟦e⟧ ρ ; the Tag N constructor named tag(v)
⟦deref r.S e⟧ ρ          = k ⇐ ⟦e⟧ ρ ; the unique v ∈ ⟦r.S⟧ D with key_r(v) = k,
                           or ↯NoSuchRow when there is none
⟦try e⟧ ρ                = Ok⟨v⟩    if ⟦e⟧ ρ = v
                         = Err⟨f⟩   if ⟦e⟧ ρ = ↯f
                         = ⊗x       if ⟦e⟧ ρ = ⊗x
```

The operands of `op` are evaluated left to right, each before the
operation. Every field of a record or constructor value is evaluated
before the value is built; when several fields abort, the outcome is
the abort of any one of them. A primitive `op` is a
function `V^n → O(V)` that [prelude.md](prelude.md) defines; a partial
one returns `↯f` outside its domain (overflow, division by zero,
conversion out of range). A checked variant is total and returns
`Option`. A key value is built only by `key` of a row or by `keyof`
from the key fields' values; no literal is a key. In a valid state
`deref` of a reference field's value always finds exactly one row,
because every reference field is constrained to name a row of its
constructor set (§7.2), and at an older version as well, except in a
bundle declaring `hide new constructors` (§9.6). A key
built by `keyof` or passed as a parameter may name no row, and `deref`
of it faults with `NoSuchRow`.

### 5.2 Guards

The guard forms evaluate their condition first and only the branch it
selects:

```text
⟦if e₀ then e₁ else e₂⟧ ρ = b ⇐ ⟦e₀⟧ ρ ; ⟦e₁⟧ ρ if b = True, ⟦e₂⟧ ρ if b = False

⟦case e of { C x_C → e_C }⟧ ρ = v ⇐ ⟦e⟧ ρ ; ⟦e_{tag v}⟧ ρ[x_{tag v} ↦ v]
```

`for x in e₁ do e₂` (§5.4) evaluates `e₂` only at the elements of `e₁`.
These are the only places where a subterm's evaluation depends on
another's value; every other form evaluates all its operands, left to
right, and the fields of one value in no fixed order (§5.1). An
implementation may evaluate a subterm where these equations do not
evaluate it, or earlier, only when the subterm is total (§3.4).
It may reorder, share, hoist and push down total subterms freely.
Partial subterms keep their order relative to each other and stay
under the guards that dominate them.

### 5.3 Binding and functions

```text
⟦let x = e₁ in e₂⟧ ρ   = v ⇐ ⟦e₁⟧ ρ ; ⟦e₂⟧ ρ[x ↦ v]
⟦λ(x : τ). e⟧ ρ        = the function v ↦ ⟦e⟧ ρ[x ↦ v]
⟦e₁ e₂⟧ ρ              = (g, v) ⇐ seq(⟦e₁⟧ ρ, ⟦e₂⟧ ρ) ; g(v)
```

Evaluation is call by value. A `let`'s bound expression and a
function's arguments are evaluated before the body, left to right,
whether or not the body uses them, and the first abort reached is the
outcome. Block bindings `let!` (§6.6) are evaluated the same way.

### 5.4 Sets

```text
⟦{}⟧ ρ                 = ∅
⟦{e}⟧ ρ                = v ⇐ ⟦e⟧ ρ ; {v}
⟦e₁ ∪ e₂⟧ ρ            = (A, B) ⇐ seq(⟦e₁⟧ ρ, ⟦e₂⟧ ρ) ; A ∪ B
⟦for x in e₁ do e₂⟧ ρ  = A ⇐ ⟦e₁⟧ ρ ; (B_v) ⇐ gather(⟦e₂⟧ ρ[x ↦ v])_{v ∈ A} ; ⋃_{v ∈ A} B_v
⟦empty e⟧ ρ            = A ⇐ ⟦e⟧ ρ ; True if A = ∅, False otherwise
⟦exists e⟧ ρ           = A ⇐ ⟦e⟧ ρ ; False if A = ∅, True otherwise
⟦e ▷ S⟧ ρ              = A ⇐ ⟦e⟧ ρ ; { v ∈ A | tag v ∈ S }
⟦only e⟧ ρ             = A ⇐ ⟦e⟧ ρ ; v   where A = {v}
⟦r.S⟧ ρ σ              = ⟦r.S⟧ D
```

Every result is a set. Two bindings that produce equal values produce
one element; a union of sets is a set. A nested collection is a field
whose value is a set; the absence of related rows is the empty set, and
no row is padded.

### 5.5 Sums and match

A relation variable is the disjoint union of its constructor relations
(§4.2), and a `case` on a row selects the arm of the row's constructor
(§5.2). Together these give the meaning of a `match` over a relation as
a disjoint union of arm queries, one per constructor relation:

```text
⟦for x in r.S do e[case x of { C x_C → e_C }_{C ∈ S}]⟧ ρ
  =  ⟦ ∪_{C ∈ S} for x_C in r.C do e[e_C] ⟧ ρ
```

for every context `e` in which `x` occurs only as the scrutinee of
such `case`s. The equation holds for comprehensions with no `group`
in which `x` is not inside the range of an aggregate or of `top`: those
range over bindings, and the union of arms merges equal values of two
arms. Each arm reads only its constructor relation. When the
arms project away the constructor, equal values from two arms are one
element of the result. Aggregates in an arm range over whatever their
own binder names; an aggregate over a whole match ranges over the
match's result.

### 5.6 Constraint-only forms

In the body of a constraint, evaluated against a pre-state `D`, a
normalised change set `Δ` and the post-state `D'` (§7.1):

```text
⟦r.S⟧           = ⊎_{C ∈ S} D'(r.C)
⟦inserted r.S⟧  = ⊎_{C ∈ S} Δ⁺(r.C)
⟦deleted r.S⟧   = ⊎_{C ∈ S} Δ⁻(r.C)
⟦prior r.S⟧     = ⊎_{C ∈ S} D(r.C)
```

### 5.7 Aggregates

An aggregate `α` is given by an argument type `τ_α`, a carrier `M`, a
lift `ι : τ_α → M`, a combining operator `⊕ : M × M → O(M)`, a final map
`φ : M → O(ρ_α)`, and optionally an identity `ε ∈ M`. With an identity
it is a monoid aggregate, without one a semigroup aggregate. The
aggregates and their signatures are listed in
[queries.md](queries.md#10-aggregates); user-defined aggregates in
[interop.md](interop.md#9-user-defined-functions) take the
same form. Examples:

| Aggregate | `M` | `⊕` | `ε` | `φ` |
| --- | --- | --- | --- | --- |
| `count` | `I64` | `+` | `0` | identity |
| `sum` over `I64` | the integers | exact `+` | `0` | the sum, or `↯ArithmeticOverflow` outside `I64` |
| `sum` over `Decimal s` | the decimals of scale `s` | exact `+` | `0` | the sum, or `↯ArithmeticOverflow` beyond 38 digits |
| `min`, `max` over ordered `T` | `T` | the lesser, the greater | none | identity |

An aggregate ranges over bindings, not over projected values:

```text
⟦agg α (x in e₁ . e₂)⟧ ρ =
  A ⇐ ⟦e₁⟧ ρ ;
  (m_v) ⇐ gather(⟦e₂⟧ ρ[x ↦ v])_{v ∈ A} ;
  B = ⦃ ι(m_v) | v ∈ A ⦄ ;
  if A = ∅:   φ(ε)            monoid
              None            semigroup over Set τ
  otherwise:  m ⇐ fold_⊕ B ; φ(m), wrapped in Some for a semigroup over Set τ
```

The range `e₁` may be any set expression, and its bindings are the
elements of that set. A range built by `select` has already merged equal
values, so `count(a for a in (from p in posts select p.author))` counts
distinct authors. `sum` and `avg` refuse a generator whose source,
after normalisation (§5.11), is a `select` comprehension that is not
key-preserving: one whose select is not exactly the binder of a
generator over a relation or a keyed set. A parameter of type `Set T`
is accepted
([queries.md §10.1](queries.md#101-aggregates-range-over-bindings)).

`B` is a multiset: two bindings with equal `e₂` contribute twice.
An integer or decimal `sum` is the exact sum, checked against its type once, by
`φ`, so its outcome does not depend on the fold order. `fold_⊕` combines the elements of `B` in some order, faulting as soon
as one `⊕` faults. When `⊕` is associative and commutative on `M` the
order does not matter. When it is not (floating-point addition), any
order is a conforming result. An aggregate over `Set⁺ τ` never sees an
empty range and returns `ρ_α` directly.

### 5.8 Grouping

```text
⟦group (x in e₁) by e₂⟧ ρ =
  A ⇐ ⟦e₁⟧ ρ ;
  (k_v) ⇐ gather(⟦e₂⟧ ρ[x ↦ v])_{v ∈ A} ;
  { { key: k, rows: { v ∈ A | k_v = k } } | k ∈ { k_v | v ∈ A } }
```

Each group's `rows` is nonempty, since its key was produced by one of
its rows. Grouping by `tag x` groups by constructor.

### 5.9 Top-n and presentation

For an order key list `k̄ = (k₁ d₁, …, k_j d_j)`, `u ≺ v` holds when the
first `k_i` on which `u` and `v` differ puts `u` first: lower for `asc`,
higher for `desc`, by the order of
[model.md §10](model.md#10-order).

```text
⟦top n by k̄ (x in e₁)⟧ ρ =
  c ⇐ ⟦n⟧ ρ ; A ⇐ ⟦e₁⟧ ρ ; (k̄_v) ⇐ gather(seq(⟦k_i⟧ ρ[x ↦ v]))_{v ∈ A} ;
  ↯NegativeCount if c < 0, else { v ∈ A | #{ u ∈ A | u ≺ v } < c }
```

Because `k̄` is injective on `A`, `≺` is a total order on `A`, the
result has exactly `min(c, |A|)` elements, and it does not depend on
evaluation order. The result is a set; it carries no order.

Ordering is not a kernel form. `order by k̄`, `take n` and `after c`
apply only to the final result of a statement and denote a sequence:

```text
present(A, k̄, n, c) = ↯NegativeCount if n < 0,
                      else the first n elements of { v ∈ A | c ≺ v }, listed by ≺
```

`take` without `after` omits the filter; `order by` alone lists every
element. `order by` alone does not need `k̄` to be injective: elements
equal on every key are listed in an unspecified order. `take` and
`after` need an injective `k̄`.

### 5.10 Fixpoints

```text
⟦fix X₁ = e₁, …, Xₙ = eₙ in e⟧ ρ = ⟦e⟧ ρ[X₁ ↦ S*₁, …, Xₙ ↦ S*ₙ]
  where F(S̄) = (⟦e₁⟧ ρ[X̄ ↦ S̄], …, ⟦eₙ⟧ ρ[X̄ ↦ S̄]),
        S̄₀ = (∅, …, ∅),  S̄_{i+1} ⇐ seq F(S̄_i),
        S̄* = S̄_m for the least m with S̄_{m+1} = S̄_m
```

The relations of one `fix` are computed together: each step evaluates
every body on the previous tuple. If some step aborts, the `fix`
aborts. Under the stratification condition below `F` is monotone in the
componentwise order, so the chain `S̄₀ ⊆ S̄₁ ⊆ …` ascends and `S̄*` is the
simultaneous least fixpoint of `F`. When the least fixpoint is finite
the chain reaches it. When it is infinite the chain never ends, and the
statement is rejected with `StatementLimit` when it exceeds its limits
([catalog.md §8.3](catalog.md#83-limits-and-deadlines)).

A `fix` is bounded when no constructed value derived from a relation
of its own stratum can become an element of one of its relations. A
value is constructed by a constructing primitive, an aggregate or
`fresh` applied to a value derived from such a relation, or by a record
or constructor value holding one. It counts only where it can become
an element of a relation of the stratum: a constructed value used only
in a condition, a comparison or an aggregate key does not count. A
constructing primitive is one whose result may be a value that is
neither one of its arguments nor a component of one: arithmetic, text
and byte building, conversions, time arithmetic. Comparisons, `==`,
`!=`, boolean operations and other primitives returning `Bool` are not
constructing, nor are the projections returning one of their arguments
or a component of one (`start`, `end`, `unwrap`, `value`). So a
reachability `fix` that compares keys is bounded, and so is one that
compares records built from its elements:

```adt fragment
-- given me: Key users
fix a = (from u in users select key u)
      union (from x in a where { k: x } != { k: me } select x)
in a
```

A backlink over an `Option` reference is bounded for the same reason:
the `Some` it builds is only compared. Every value a bounded `fix`
adds is built from finitely many values present before it runs, so its
least fixpoint is finite and it is total. A `fix` that is not bounded
is partial (§3.4).

Stratification. The relations bound by one `fix` form one stratum. The
positive positions of each `X_j` in a body `e_i`, and the positive
conditions, are defined by:

- `e_i` itself is positive;
- if `e' ∪ e''` is positive, so are both operands;
- if `e' ▷ S` is positive, so is `e'`;
- if `for y in e' do e''` is positive, so are `e'` and `e''`;
- if `if c then e' else e''` or a `case` is positive, so are its
  branches; its scrutinee is not positive, and its condition `c` is a
  positive condition when `e''` is `{}`;
- in a positive condition, `exists e'` makes `e'` positive, and
  `if p then q else False` and `if p then True else q` make `p` and `q`
  positive conditions;
- if `let y = e' in e''` is positive, so is `e''`, but not `e'`;
- if an inner `fix Ȳ = ē' in e''` is positive, so are its bodies `ē'`
  and `e''`.

Every other position is not positive: the argument of `empty`, any
other condition, the range and body of an aggregate, the source and key
of `group` and `top`, a field of a record or constructor value, a
primitive's argument, a function body or argument. `exists` at a
positive condition is positive; `empty`, and `not exists`, which
desugars to it, are negation. Every `X_j` must occur only at positive
positions of every body of its `fix`. A relation defined by an outer
`fix`, already computed, may be used anywhere, including under `empty`
and in aggregates: negation and aggregation reach only lower strata. A
program with a cycle through a non-positive position is refused at
compile time.

Mutually recursive relations are bound by one `fix`; a relation of a
stratum may not be negated or aggregated by another relation of the
same stratum. A `match` inside a fix body stays positive: its arms
range over disjoint constructor relations.

### 5.11 Normalisation

Functions are values of the kernel only while a term is being
normalised. Every registered query, schema function, constraint and
statement is a closed term whose free variables have first-order
types. It denotes what its normal form denotes under these rules,
where `e'` total means `e'` is total (§3.4):

```text
(λ(x : τ). e) e'                       → e[x := e']           e' total
(λ(x : τ). e) e'                       → let x = e' in e      otherwise
let x = e' in e                        → e[x := e']           e' total
for x in {} do e                       → {}
for x in {e'} do e                     → e[x := e']           e' total
for x in (e₁ ∪ e₂) do e                → (for x in e₁ do e) ∪ (for x in e₂ do e)
for y in (for x in e₁ do e₂) do e      → for x in e₁ do (for y in e₂ do e)
for x in (if c then e₁ else e₂) do e   → if c then (for x in e₁ do e) else (for x in e₂ do e)
{ …, l: e, … }.l                       → e                    the other fields total
(C { …, l: e, … }).l                   → e                    the other fields total
case C { … } of { …; C x → e; … }      → e[x := C { … }]      the fields total
```

and schema functions are unfolded at their calls. Since no function is
recursive and `fix` binds only sets, normalisation terminates. Its
result contains no λ and no application; a `let` remains only where its
bound expression is partial. Each step preserves the denotation of
§5.3, aborts included.

Rulings: query/union-all, query/ctor-relations, query/construct, model/dup-arms/dedupe, lang/collection/sets, lang/collection/set-agg/over-binders, lang/empty-agg/nonempty, query/agg-scope/whole, lang/group/into, lang/group/ctor-key/tag-type, lang/nested-order/top-n, lang/order/result-only, lang/keyset, lang/recursion/fix, lang/recursion/negation/stratified, lang/abstraction/higher-order, lang/abstraction/ho-recursion/first-order-recursion, lang/outer/nested, lang/results/nested, types/fault/abort, types/fault/reorder/ordered-guards, types/fault-forms/checked, types/fault-forms/try, types/int-overflow/trap, project/oracle/float/tolerance, derived/transition/change-visible, spec/tiers/three-tiers, spec/field-faults/any, spec/take-negative/fault, spec/fix/restriction/positive, spec/fix/bounded/element-flow, spec/fix/projection/not-constructing

## 6. Writes

### 6.1 Change sets

A change set over `Σ` is a pair of families

```text
Δ = (Δ⁺, Δ⁻),   Δ⁺(r.C), Δ⁻(r.C) ⊆ values with tag C, for every r.C
```

`Δ⁺(r.C)` are the rows inserted into `r.C` and `Δ⁻(r.C)` the rows
deleted from it. A write term denotes a set of effects, from which its
change set is read off:

```text
Eff ::= ins(r, v)          insert row v into r
      | del(r, v)          delete row v of r
      | rep(r, v, v')      replace row v of r by v', key_r(v) = key_r(v')
      | kill(r, k)         delete every row of r with key k (derived by cascade, §6.4)
```

The key of an effect is `key_r(v)` (for `rep`, of either row; for
`kill`, `k`).

### 6.2 Denotation of write terms

`⟦w⟧ ρ σ ∈ O(finite sets of Eff)`:

```text
⟦skip⟧ ρ                     = ∅
⟦insert r e⟧ ρ               = A ⇐ ⟦e⟧ ρ ; { ins(r, v) | v ∈ A }
⟦delete r e⟧ ρ               = A ⇐ ⟦e⟧ ρ ; { del(r, v) | v ∈ A ∩ ⟦r⟧ D }
⟦replace r e₁ e₂⟧ ρ          = (v, v') ⇐ seq(⟦e₁⟧ ρ, ⟦e₂⟧ ρ) ;
                               ⊗KeyChanged   if key_r(v) ≠ key_r(v')
                               ∅             if v = v' or v ∉ ⟦r⟧ D
                               { rep(r, v, v') } otherwise
⟦w₁ ∥ w₂⟧ ρ                  = (E₁, E₂) ⇐ gather(⟦w₁⟧ ρ, ⟦w₂⟧ ρ) ; E₁ ∪ E₂
⟦for x in e do w⟧ ρ          = A ⇐ ⟦e⟧ ρ ; (E_v) ⇐ gather(⟦w⟧ ρ[x ↦ v])_{v ∈ A} ; ⋃ E_v
⟦if e then w₁ else w₂⟧ ρ     as §5.2
⟦case e of { C x → w_C }⟧ ρ  as §5.2
⟦let x = e in w⟧ ρ           = v ⇐ ⟦e⟧ ρ ; ⟦w⟧ ρ[x ↦ v]
⟦fail e⟧ ρ                   = t ⇐ ⟦e⟧ ρ ; ⊗UserError(t)
```

Every read in a write term reads the snapshot `D`; no write sees
another write's effects except through a `let!` binding (§6.6). The
order of `∥` operands has no effect on the change set, nor on which
abort is the outcome when both operands abort. `fail` rejects
the statement; it is not a fault, and `try` does not catch it. A
`replace` whose new value has another constructor is a retag: the row keeps its key and header
fields and moves between constructor relations.

### 6.3 Combining effects

The change set of a set of effects `E` is the union of its effects:

```text
killed(E) = { (r, k) | kill(r, k) ∈ E }
Δ⁺(r.C) = { v | ins(r, v) ∈ E or rep(r, _, v) ∈ E, tag v = C, (r, key_r v) ∉ killed(E) }
Δ⁻(r.C) = { v | del(r, v) ∈ E or rep(r, v, _) ∈ E, tag v = C }
        ∪ { v ∈ D(r.C) | (r, key_r v) ∈ killed(E) }
```

Normalisation removes from both `Δ⁺(r.C)` and `Δ⁻(r.C)` every row in
both. Several writes may reach one key; after the union
([writes.md §1.4](writes.md#14-combining-writes)):

- equal effects are one effect, so several deletes of the same row,
  explicit or derived by `cascade`, are one delete;
- a key whose pre-state row is in `Δ⁻` and for which a row is in `Δ⁺`
  is a replacement of that row: an update when the inserted row has
  the same constructor, a retag when it has another. A delete and an
  insert of the same key mean the same as the update or retag that
  produces the inserted row;
- two different rows in `Δ⁺` with one key, two updates of one row to
  different values included, fail the statement with `KeyViolation`,
  reported at the statement boundary (§6.6);
- an `ins(r, v)` whose key is the key of a row `u₀ ∈ D(r)` that no
  `Δ⁻(r.C)` holds fails the statement with `KeyViolation` (§6.6), even
  when `v = u₀`: an insert never replaces a row.

### 6.4 Delete actions

For the written effects `W`, let `Δ_W` be their change set (§6.3) and
`D_W = apply(D, Δ_W)` (§6.5). A reference value `u.f` names `(r, k)`
when `f : Key r.S` and `u.f = k`, or `f : Option (Key r.S)` and
`u.f = Some⟨k⟩`. The gone keys are the least set `G` such that:

```text
(r, key_r(u₀)) ∈ G   for every u₀ ∈ D(r) such that D_W(r) has no row of key key_r(u₀)
(r', key_r'(u₀)) ∈ G for every u₀ ∈ D(r') ∩ D_W(r') with a field f, action cascade, where u₀.f names a key in G
```

Delete actions reach only rows of `D` that the written effects neither
replace nor delete. For a row `u₀ ∈ D(r'.C) ∩ D_W(r'.C)` with
`(r', key_r'(u₀)) ∉ G`, let `F(u₀)` be the reference fields of `C`,
header fields included, where `u₀.f` names a key in `G`. Every such
`u₀` with `F(u₀)` not empty takes one new value from every action in
`F(u₀)`:

```text
new(u₀) = fill_r'(D⟨key and header fields of u₀, payload⟩) with g = None for every g ∈ F(u₀) with action clear that D has
                                          if some f ∈ F(u₀) has action retag D {payload}
        = u₀ with g = None for every g ∈ F(u₀) with action clear
                                          otherwise, if some f ∈ F(u₀) has action clear
        = none                            otherwise
```

All `retag` actions among a type's fields are one action, with one
target and one payload
([writes.md §7.1](writes.md#71-declaring-an-action)), so `D` and its
payload are determined. The derived effects are

```text
A = { kill(r, k) | (r, k) ∈ G } ∪ { rep(r', u₀, new(u₀)) | u₀ as above, new(u₀) ≠ none }
```

and the statement's effects are `E* = W ∪ A`. `fill_r'` of a `retag`
action's value fills the fields of `D` the payload omits, as an insert
into `r'.D` does, drawing from `γ`.

Consequences, matching [writes.md §7.2](writes.md#72-when-an-action-runs):

- a row whose key is in `G` is deleted, even when a `retag` or `clear`
  action also reaches it: `kill` removes every inserted row of its key;
- several `clear` fields of one row, and a `retag` with `clear` header
  fields, give the row one new value;
- a row the statement inserts, whose key has no row in `D`, is reached
  by no action: if it references a gone key, its reference constraint
  (§7.2) rejects the statement, whatever the action;
- a row the statement replaces is reached by no action and is judged
  by its new value: if that value references a gone key, its
  reference constraint rejects the statement;
- an update or a retag of the target never makes its key gone and fires
  no action;
- `restrict` derives nothing; the reference constraint (§7.2) rejects
  the statement if a referrer of a gone key remains.

### 6.5 Application

```text
apply(D, Δ)(r.C) = (D(r.C) ∖ Δ⁻(r.C)) ∪ Δ⁺(r.C)     for every r.C at once
```

Application is one multiple assignment: every constructor relation of
every relation variable takes its new value together, and no state in
which some have been assigned and others not exists or is observable.

### 6.6 Statements

```text
⟦let! x = e in b⟧ ρ σ    = v ⇐ ⟦e⟧ ρ σ ; ⟦b⟧ ρ[x ↦ v] σ
⟦(w, e)⟧ ρ σ             = (E, v) ⇐ seq(⟦w⟧ ρ σ, ⟦e⟧ ρ σ) ; (E, v)
```

`let!` evaluates its expression once, in the snapshot, with its
`fresh` instances drawn from `γ`. A name bound to the rows an insert
would insert is how a later write or the result refers to generated
keys. The result `e` is evaluated in the snapshot; it sees pending rows
only through such names.

The sequencing above fixes the outcome only when one item of a block
aborts. When more than one `let!` binding, write or result of a block
would abort, the outcome is any one of those aborts, a freedom beside
the one §1.1 states for the bindings of a set.

The outcome of `transact b` in context `σ = (D, Σ, v, γ)`:

```text
run(s, σ) =
  (E, res) ⇐ ⟦b⟧ ∅ σ ;
  E*  = E with the effects its delete actions derive (§6.4) ;
  Δ   = change set of E* ;  Δ* = normalise(Δ) ;  D* = apply(D, Δ*) ;
  ⊗NotOpen         if some v ∈ Δ*⁺(r.C) with C not open for writes in stage(Σ)
                   has no row of its key in D(r.C): an insert or a retag into C;
                   or if some v ∈ Δ*⁺(r.C) holds at a field path p a value w
                   of a value-level constructor E not open for writes, and the
                   row of its key in D(r), if any, does not hold a value equal
                   to w at p
  (R_c)_{c ∈ cons(Σ)} ⇐ gather(⟦c⟧(D, Δ*, D*))_{c ∈ cons(Σ)} ;
  viol = { c ∈ cons(Σ) | R_c ≠ ∅ }
       ∪ { the key of r | some ins(r, v) ∈ E* with key_r(v) = key_r(u₀),
                          u₀ ∈ D(r), u₀ ∉ Δ⁻(r.C) for every C } ;
  c₀   = the c ∈ viol with the least name ([constraints.md §7.1](constraints.md#71-constraint-names)) ;
  ⊗KeyViolation    if viol ≠ ∅ and c₀ is the key of a relation variable
  ⊗Violated(c₀, R_c₀) if viol ≠ ∅ otherwise
  (res, Δ*, D*)    otherwise
```

An update that keeps a row in a deprecated constructor, or keeps a
field's value in a deprecated value-level constructor, is not refused.
The rejection carries one violated constraint and its report rows; the
caller sees the rows its grants allow
([constraints.md §7](constraints.md#7-checking-and-violation-reports)).

An abort anywhere in this computation is the statement's outcome, and
no part of its change set is applied. An abort while evaluating any
constraint takes precedence over every violation. The key and constraint tests are
made at the statement boundary, on `D*`. A statement that succeeds
replaces `D` by `D*` for the statements after it in its transaction.
`D` is the snapshot: a test that would meet a row committed by another
transaction after the snapshot is not decided here; it ends the
transaction with a serialization failure (§8.4). `NotOpen`,
`KeyViolation`, `Violated`, `KeyChanged` and `UserError` are
constructors of `RejectReason` ([catalog.md](catalog.md#4-error-sums)).

### 6.7 Inserts with rejects

`insert r (for x in e₀ do e) rejects into j` is given on the records
of its one generator's source `e₀`, a source relation or an external
source. `P` is the set of those records, each `p` with its position
`n_p` in the source's stated order, from 1, its source name `s_p` and
its text `t_p` ([interop.md §4.5](interop.md#45-rejects)). `conv(p)` is
the set of rows `e[x := p]` computes, or `✗m` when `p` does not
convert to the row type the source names, or its computation reaches
`reject m`, `m` the error. `ℓ` is `fresh` (§2.5): one value per
statement, distinct from every `load` value issued before, drawn from
`γ`. With `q` for `for x in e₀ do e`:

```text
⟦insert r q rejects into j⟧ ρ σ =
  (o_p)_{p ∈ P} ⇐ gather(conv(p))_{p ∈ P} ;
    { ins(r, fill_r(v)) | p ∈ P, o_p a set of rows, v ∈ o_p }
  ∪ { ins(j, { load: ℓ, line: n_p, source: s_p, raw: t_p, reason: m }) | p ∈ P, o_p = ✗m }

⟦insert r q⟧ ρ σ = ⊗BadInput    if some o_p = ✗m, as above
```

Any other abort of `conv(p)` is the outcome of the write. Both sets of
rows are effects of the one statement, so a constraint violation
rejects the statement with its rejects.

Rulings: write/primitive/delta, write/surface/txdata, write/unit/multi, write/retag/assign, write/retag/old/in-scope, write/transact/block, write/insert/returns/rows, write/bulk/from-query, write/delete-action/cascade, write/delete-action/restrict, write/delete-action/retag-referrer, fk/retag/recheck, model/deferred-check, constraint/timing/multi-assign, types/keygen/promise/distinct, spec/writes/insert-existing/violation, spec/writes/action-compose/one-value, spec/writes/action-reach/pre-state, spec/writes/abort-choice/any-abort, spec/writes/violation-name/canonical-least, spec/writes/rejects/per-row, spec/writes/rejects-source/one-source, spec/writes/action-identical/identical-only, spec/writes/action-own-writes/statement-wins, spec/evolution/deprecated-value/same-path

## 7. Constraints

### 7.1 Satisfaction

A constraint `c` is a closed kernel query. It is evaluated on a
transition `(D, Δ, D')` with `D' = apply(D, Δ)` (§5.6) and holds when
it returns the empty set:

```text
holds(c, D, Δ, D')  ⇔  ⟦c⟧(D, Δ, D') = ∅
valid(D)            ⇔  ∀ c ∈ cons(Σ). holds(c, D, ∅, D)
```

On the empty change, `inserted` and `deleted` are empty and `prior r.S`
is `r.S`. The rows `⟦c⟧` returns are the violation report. An abort
while evaluating `c` is the statement's outcome.

### 7.2 Structural constraints

Keys, uniques and references are constraints in this form, generated
from the schema:

```text
key of r:
  for x in r do for y in r do
    if key_r(x) == key_r(y) and x != y then {{ a: x, b: y }} else {}

unique (f̄) in S of r:
  for x in r.S do for y in r.S do
    if x.f̄ == y.f̄ and key_r(x) != key_r(y) then {{ a: x, b: y }} else {}

unique (f.C.g) in S of r, f a field of a sum type with constructor C:
  for x in r.S do for y in r.S do
    if tag(x.f) == C and tag(y.f) == C and x.f.g == y.f.g
       and key_r(x) != key_r(y) then {{ a: x, b: y }} else {}

reference field f : Key r.S of constructor C of T_r':
  for u in r'.C do
    if empty(for t in r.S do if key_r(t) == u.f then {t} else {}) then {u} else {}

reference field f : Option (Key r.S) of constructor C of T_r':
  for u in r'.C do case u.f of {
    None ξ → {} ;
    Some s → if empty(for t in r.S do if key_r(t) == s.value then {t} else {}) then {u} else {} }

row check on r.S with condition p (self the row):
  for x in r.S do if p[self := x] then {} else {x}

domain of newtype N with predicate p, for every constructor C of every r:
  for x in r.C do
    if exists(for y in occ_N(x) do if p[self := y] then {} else {y}) then {x} else {}
```

In a unique over a path into a value-level sum, only rows whose field
holds the named constructor take part: `unique (status.Paid.ref)`
constrains no row whose `status` is `Draft` or `Sent`.

`occ_N(x)` is the set of values of `N` occurring in `x` at any depth,
built from `x` by field access and `case` over its static type. A
domain is checked only on stored rows: constructing a value of `N`
never faults on its predicate.

A violation of the key constraint rejects the statement with
`KeyViolation`, of any other with `Violated`. Exclusivity and totality
of constructors need no constraint: a row is
one constructor value, and `r` has no rows outside its constructor
relations. A reference to `r.S` whose target is retagged to a
constructor outside `S` violates its reference constraint; a retag
fires no delete action.

### 7.3 Statement boundaries

Every constraint holds at every statement boundary:

```text
D valid ∧ run(s, σ) = (res, Δ*, D*)  ⟹  D* valid
```

because `run` refuses every statement whose transition violates a
constraint (§6.6). No constraint is checked inside a statement, and none
is deferred to commit.

### 7.4 Incremental checks

A constraint is admitted only with an index-bounded incremental check
([constraints.md §5](constraints.md#5-admission)). Admission decides
which constraints a schema may hold; it never changes what a
constraint means (§7.1).

Rulings: constraint/form/empty, constraint/assertion, constraint/timing/multi-assign, constraint/enforce/delta, constraint/admit/bounded, derived/transition/change-visible, surface/constraint/check, surface/constraint/denial, constraint/scope/ctor-set, fk/to-sum, fk/to-ctor, fk/to-subset, fk/retag/recheck, sum/disjoint-total, btree/sum-field/ctor-field, spec/tiers/three-tiers

## 8. Isolation

### 8.1 Transactions and histories

A transaction is a sequence of statements `s₁ … s_m`. A transact block
or a registered transaction is a transaction of one statement; an
interactive transaction is one statement per call of a registered
query or transaction and one per item of a call of ad hoc text
([transactions.md §2](transactions.md#2-statements)). A migration
applies as a transaction too, of one statement (§9.5). A history is the set of
transactions the database has run, each committed or aborted, with the
state each committed transaction installed. Only committed
transactions change the state.

### 8.2 Items, reads and writes

The items are the rows `(r, k)` of each relation variable, named by key,
whatever their constructor. A committed transaction whose change set
holds `k` in `Δ*⁺(r.C)` or `Δ*⁻(r.C)` writes `(r, k)` and installs a
version of it: a row of some `r.C`, or its absence. A retag installs one
version, in the new constructor relation.

A statement reads the constructor relations its denotation depends
on; a read of `r.S` is a read of every `r.C` with `C ∈ S`. Constraint
checks are reads of the statement that triggers them. The schema version is an item too:
every statement reads the version it runs at, and applying a migration
writes it, so a writer at version `n - 1` is serialized against the
migration that produces `n`.

### 8.3 Tests against later commits

A statement is evaluated on its snapshot. When one of its tests meets
the effect of a transaction that committed after the snapshot, the
transaction fails with a serialization failure, never with a key
violation and never by re-evaluating on the newer row
([transactions.md §6.3](transactions.md#63-waiting-on-a-key)):

- a new row whose key or unique value equals that of a row committed
  after the snapshot;
- an upsert whose conflicting row was committed after the snapshot;
- a test that meets an uncommitted write of a concurrent transaction to
  a key or unique value it tests waits for that transaction; when it
  commits, the waiter's transaction fails with a serialization failure,
  and when it aborts, the statement proceeds as if it had never run.

Duplicate keys among a statement's own inserted rows, and an insert of
a key the snapshot holds, are `KeyViolation` (§6.6).

### 8.4 Guarantee

For every history the database produces there is a total order
`T₁ < T₂ < … < T_n` of its committed transactions alone such that
running each statement of each `T_i` by `run` (§6.6), or by the
translated run of §9.6 for a statement at an older version, in that
order, from the initial state, with the same supplies `γ`, gives every
statement the outcome the database returned to it, and leaves the
state the database holds. This is the guarantee, and the only isolation
level. Aborted transactions take no part in it.

A transaction whose commit would leave no such order does not commit;
it fails with a serialization failure and may be retried
([transactions.md](transactions.md#55-retry)). A transaction may also
fail with one although such an order exists
([transactions.md §6.1](transactions.md#61-serialization-failures)).

The order extends real time on the primary: when `T`'s commit was
acknowledged before `T'` began, `T < T'`. Reads on a replica keep the
guarantee [replication.md §1.2](replication.md#12-reads-on-a-replica)
states for them, which ties their snapshot to commit positions and not
to real time.

Two consequences: concurrent transactions cannot jointly violate a
constraint that each satisfies alone, since each statement's check is a
read; and a retag is ordered against every concurrent statement that
references, reads or retags the same row.

Rulings: engine/serializable-only, engine/ssi, engine/retag-fk/by-isolation, mvcc/ctor-tagged, constraint/concurrent/serial-checks, write/transact/interactive, write/transact/registered-txn, engine/transactional-ddl, spec/txn/isolation-statement/serial-order, spec/txn/real-time/primary, spec/txn/call-atomic/item-statements, spec/txn/may-abort/may, spec/writes/insert-existing/violation, spec/evolution/migration-statement/one-statement

## 9. Schema evolution

### 9.1 Versions

The schema at version `n` is `Σ_n = op_n(… op_1(Σ_0))`, the fold of the
operator log up to its `n`th entry. The log is linear and only grows;
its position is the version. Every operator is typed: it applies to a
schema of a given shape and produces another, and one that does not
apply is refused.

### 9.2 An operator as maps

An operator `op : Σ → Σ'` denotes:

```text
fwd_op : St(Σ) ⇀ St(Σ')                   the data map applied with the operator
bwd_op : St(Σ') → St(Σ)                   the inverse map: Σ' state seen at Σ
put_op : St(Σ') × St(Σ) ⇀ St(Σ')          a Σ write carried into a Σ' state
π_op   : St(Σ) → St(Σ)                    the declared loss; the identity unless declared
```

`St(Σ)` is every state over `Σ` (§4.2), valid or not; `DB(Σ) ⊆ St(Σ)`
is the valid ones. `bwd` yields a state of `Σ`, not necessarily a
valid one: rows that violate a constraint of `Σ` dropped since map
back unchanged. `fwd_op` is partial: an operator whose preconditions
fail on the state it applies to is refused (removing a populated
constructor). Every precondition is a predicate read (§8.2) of the
migration's transaction. The laws:

```text
(L1)  bwd(fwd(D)) = π(D)              for D ∈ dom fwd
(L2)  bwd(put(D', D)) = D             where put(D', D) is defined
(L3)  put(D', bwd(D')) = D'
(L4)  π ∘ π = π
(L5)  c ∈ cons(Σ) is carried to c' ∈ cons(Σ')
```

`fwd` need not yield a valid state: the constraints of the final
schema are checked once, on the state the whole migration produces
(§9.5).

An operator is lossless when `π` is the identity. A lossy operator
states its loss with the operator, as the projection `π` of the
information `fwd` forgets ([evolution.md](evolution.md#4-operators)).
An operator that cannot carry a constraint is refused.

The operator's inverse `op⁻¹ : Σ' → Σ` is the operator with
`fwd_{op⁻¹} = bwd_op` and `bwd_{op⁻¹} = fwd_op`. Its own preconditions
and loss follow from these maps: the inverse of an operator whose `fwd`
is total and whose `bwd` drops information is lossy.

Operators that move data act row by row: there is a key-preserving map
`φ_op` from rows of `r` at `Σ` to rows of `r` at `Σ'` with

```text
fwd(D)(r) = { φ(v) | v ∈ D(r) }
bwd(D')(r) = { φ⁻¹(v') | v' ∈ D'(r), v' in the image of φ }
put(D', D)(r) = { φ(v) ⊕ v'_k | v ∈ D(r) } ∪ { v' ∈ D'(r) | v' not in the image of φ }
                 v'_k the row of D'(r) with v's key, if any
```

where `φ(v) ⊕ v'` keeps from `v'` the fields of `Σ'` that `φ⁻¹` drops,
when `v'` has the constructor of `φ(v)`. When `D'` has no row with
`v`'s key (a row inserted at `Σ`), or that row has another constructor,
each such field takes the relation's default for it in that
constructor at the current version (`Σ_m` of §9.6), and without one
the operator's fill: the fill of
an `add field`, the fill of a field moved up to the header.
Rows of `D'` outside the image of `φ` (rows of a constructor added
after `Σ`) are invisible at `Σ` and kept by `put`. An operator on a
parametric sum maps every instantiation by the same `φ`.

### 9.3 The operators

| Operator | `φ` (`fwd`, row by row) | `bwd` | Loss |
| --- | --- | --- | --- |
| add constructor `T.C` | identity; `r.C` starts empty; a value-level `C` starts declared, so no stored value holds it | hides rows of `C` | none |
| remove constructor `T.C` | identity; requires `r.C` empty | `r.C` empty | none |
| rename constructor `C` to `C'` | `C⟨…⟩ ↦ C'⟨…⟩` | `C'⟨…⟩ ↦ C⟨…⟩` | none |
| split `C` into `C₁ \| … \| C_k` by a match | the routing match, per row | the inverse routing | none when the routing is invertible, else declared |
| merge `C₁ \| … \| C_k` into `C` | `C_i⟨…⟩ ↦ C⟨discriminator: C_i, …⟩`, or the `by match` routing | route by the discriminator, or invert the routing | none when the routing is invertible as for split, else declared |
| nest constructors under an intermediate sum, or unnest | re-wrap or unwrap; leaf constructor kept | the converse | none |
| add a field with fill `e` | `f = e(v)` | drop `f` | none |
| drop a field | drop `f` | `f` from the fill the inverse states | the values of `f` |
| move a field up to the header with fill `e` | constructors lacking `f` get `f = e(v)` | drop `f` where it was absent | none |
| move a header field down to constructors `S` | `f` becomes a field of the constructors of `S` | the inverse | the values of `f` in the constructors outside `S` |
| rekey `r` with `carry` fields | rows of `r` unchanged; every carried reference rewritten from the old key of its target to the new key; a relaxing rekey keeps the old key enforced as a unique while its drop is pending | the converse | none while the old key's fields remain |
| add, drop or change a named set | identity | identity; a reader pinned earlier reads the set's membership at its version | none |
| add or drop a unique, an index, a constraint, a check, a computed field | identity; adding a unique, constraint or check requires the migration's final state to satisfy it; a dropped unique, constraint or check stays enforced while its drop is pending | identity | none |
| define, redefine or drop a function | identity | identity | none |
| enable, disable, deprecate | identity; changes `stage(Σ)` of `r.C`, or of a value-level `T.C` for every field; `disable r.C` requires `r.C` empty, and `disable T.C` that no stored value holds `C` | identity | none |
| install an extension, adopt zone rules | identity on stored values; zoned values keep their civil time and zone | identity | none |
| renormalise text to a new Unicode version | renormalise every text value | identity | values that become equal |
| script | the script's change set | identity | not invertible |

The routing of a split is an exhaustive match over the old
constructor's row, checked as any match. A routing is invertible when
every arm carries each field whole into the new value or fixes it to a
literal or a nullary constructor. Routing arms and retag-action payloads
may omit key and header fields, which carry over. A split re-scopes the uniques
on `C` through its routing, and a reference typed `Key r.C` is retyped
to `Key r.(C₁ | … | C_k)`. A populated constructor goes away by a merge
that replaces it, or by a `deprecate`, a script that retags its rows
out and a later remove; remove itself never deletes rows. Split and unnest products take their source's stage, per
relation variable. The inverse of `enable` is `disable` when the
constructor was declared and `deprecate` when it was deprecated, and
`deprecate` applies only to an enabled constructor. Retiring a bundle is a command, not an
operator, and changes no schema ([applications.md](applications.md)).

A script changes data, not types: `Σ' = Σ`, and `fwd` is the state
after its change set, evaluated on the state the operators before it
in the migration produce. Translation across a script is the identity, so
a reader pinned before it reads the data the script wrote, through the
maps of the other operators. A script is never refused or delayed for
pinned bundles. It satisfies none of L1 to L3 and has no inverse;
`revert` of a migration containing one is refused.

### 9.4 Undo

`revert n` appends, in reverse order, the inverses of the operators of
migration `n`. Each inverse `op⁻¹` restores the state before `op`, up to the
declared loss `π_op`. It
names objects by id, is computed against `Σ` before `op`, and is
resolved to current names when it applies. The inverse of an `arm`
clause of `add constructor` removes that arm from its query. An
inverse whose `fwd` needs values `D'` does not hold (the `add field`
inverse of a `drop field`) takes them from a `fill T.C.f = e` clause of
the revert, `e` read as an `add field` fill; without one the revert is
refused. A migration `m` after `n`
depends on `n` when an operator of `m` names an object id that `n`
created, and `revert n` is refused while such an `m` is applied. The
log keeps both; versions are never reused. A revert whose inverses are
refused on the current state is refused.

### 9.5 The log as one history

A migration is one statement of its transaction (§8.1): its operators
apply together as one schema change, producing the version its number
names. Its data map is the composite `fwd_{op_l} ∘ … ∘ fwd_{op_k}` of
its operators' maps in order, and its change set is
`diff(D, fwd(D))`. The intermediate states need not be valid. Every
constraint of the final schema is checked once, on the result, and an
added constraint is validated there, not on the state after the
operator that adds it. A staged migration is this statement,
serialized at its final commit. Operators
that change definitions (functions, constraints) and lifecycle stages
are entries of the same log as operators that change data, and are
versioned by it.

### 9.6 Version translation

For versions `n ≤ m`, let

```text
B_{n←m} = bwd_{op_{n+1}} ∘ … ∘ bwd_{op_m}     : St(Σ_m) → St(Σ_n)
P_{n←m}(D_m, D_n) = put_{op_m}(D_m, put_{op_{m-1}}(… , D_n))   composed likewise
```

A query `q` registered at version `n` and called when the database is
at version `m` means, with the schema functions, view definitions,
named-set memberships and types of `Σ_n`:

```text
⟦q⟧^{n}(D_m) = ⟦q⟧_{Σ_n}(B_{n←m}(D_m))
```

A statement `s` at version `n` runs on `D_m` as:

```text
D_n = B_{n←m}(D_m)
(res, Δ_n, D_n') ⇐ run_{Σ_n}(s, (D_n, Σ_n, n, γ))       without the delete actions and checks of §6.6
D_m'' = P_{n←m}(D_m, D_n')
(res, Δ_m, D_m') ⇐ the actions and checks of §6.6 at Σ_m on (D_m, D_m'')
```

Delete actions, constraints (with pending drops, §2.1), indexes and
stored views are those of `Σ_m`: a translated write that violates a constraint added after `n`
fails as an ordinary constraint violation. A stored view redefined
after `n` is read by evaluating its body at `Σ_n`, not from its stored
rows. A named set is read with its
membership at `n`: a reader pinned before constructor `C` joined
`identities.OAuth` reads `identities.OAuth` as the constructors it had
at `n`.

A registered query or transaction is translatable across an operator
when `P` is defined on every state its writes can produce. The forms
that make a query untranslatable, and what reading a field means, are
listed in [evolution.md §7.2](evolution.md#72-translation-is-decided-at-apply);
they decide this criterion. Every live
registered query and transaction is translated when an operator
applies, and an operator across which one is untranslatable is refused,
naming it ([evolution.md](evolution.md#7-version-translation)). A write
at a version before a constructor was removed that inserts into that
constructor is untranslatable.

`deref` of a reference field's value is total for a translated reader
other than one declaring `hide new constructors`. A constructor `C` added after
version `n` is enabled for writes only when no live reader pinned at a
version without `C` reads the whole relation variable or follows a
reference whose type admits `C`. So while such a reader is live, no row
it can reach references a row of `C`, and no `deref` it makes meets a
row that `B_{n←m}` hides. Reads of named or listed sets, translated to
their membership at `n`, do not block enabling and never reach rows of
`C`.

A constructor `C` of a value-level sum `T` is gated the same way:
`enable T.C` gates storing a value of `C` in any field, and is refused
while any live bundle pinned at a version without `C` reads a field whose
type reaches `T`. No stored value holds `C` before it is enabled (§9.3),
so while such a reader is live no field it reads holds `C`, and
`B_{n←m}` needs no case for fields.

A bundle declared `hide new constructors` never blocks enabling a
constructor of a relation-level sum. For
it, `B_{n←m}` hides the rows of every constructor added after `n`, and
its whole-relation reads and references whose type admits such a
constructor are translated to the constructors of `n`. Rows of
constructors it knows are never hidden, so a visible row may reference
a hidden one. In such a bundle, `deref` of a key whose row is hidden
faults `NoSuchRow`, and so does every path through such a reference:
`deref` is partial in a bundle declaring `hide new constructors`. Its
writes still meet hidden rows through key and unique tests.

Rulings: evolution/smo, evolution/add, evolution/remove, evolution/rename, evolution/split, evolution/merge, evolution/move-field, evolution/nest, evolution/merge-tag/discriminator, evolution/split-routing/match-route, evolution/hoist/default, evolution/remove-rows/restrict, evolution/remove-rows/merge-first, evolution/revert, evolution/scripted, evolution/in-log, evolution/log-ops, evolution/old-readers/readers-first, model/param-evolution/live, constraint/evolve/carry, constraint/evolve/icmo, constraint/evolve/policy/check, constraint/evolve/unmapped/refuse, derived/defaults/backfill/separate, types/text-eq/unassigned/pin-unicode, surface/source/op-log, surface/version/counter, surface/version/history/linear, surface/version-serving/translate, surface/version-serving/untranslatable/translate-at-apply, engine/transactional-ddl, surface/registration/order/pinned, spec/evolution/migration-statement/one-statement, spec/evolution/preconditions/predicate-read, spec/evolution/enable-inverse/prior-state, spec/evolution/revert-identity/by-id, spec/evolution/product-state/source-state, spec/evolution/translatable/semantic-via-triggers, spec/evolution/translated-fields/same-ctor, spec/evolution/bwd-validity/any-state, spec/evolution/staged-meaning/one-transaction, spec/evolution/deprecated-value/same-path, spec/evolution/product-position/first-source, spec/evolution/revert-fill/per-field, spec/tiers/three-tiers

## 10. Desugaring

Each surface form rewrites to the kernel by the rules below, applied
outside in. `⟦⟦e⟧⟧` is the kernel term of surface `e`. Fresh kernel
variables are written `ξ`. Surface forms are defined in the part named
in each table's first column.

### 10.1 Queries

| Surface ([queries.md](queries.md)) | Kernel |
| --- | --- |
| `from x₁ in e₁, …, xₙ in eₙ where p select e` | `for x₁ in ⟦⟦e₁⟧⟧ do … for xₙ in ⟦⟦eₙ⟧⟧ do if ⟦⟦p⟧⟧ then {⟦⟦e⟧⟧} else {}` |
| `from …` without `where` | as above with `p = true` |
| `r`, `r.C`, `r.(C \| D)`, `r.S` (named) | `r.ctors(T_r)`, `r.{C}`, `r.{C, D}`, `r.S` with `S` listed |
| `e.C`, `e.(C \| D)` for a query result `e` | `⟦⟦e⟧⟧ ▷ {C}`, `⟦⟦e⟧⟧ ▷ {C, D}` |
| `q₁ union q₂` | `⟦⟦q₁⟧⟧ ∪ ⟦⟦q₂⟧⟧` |
| `p and q` | `if ⟦⟦p⟧⟧ then ⟦⟦q⟧⟧ else False` |
| `p or q` | `if ⟦⟦p⟧⟧ then True else ⟦⟦q⟧⟧` |
| `not p` | `if ⟦⟦p⟧⟧ then False else True` |
| `true`, `false` | `True`, `False` |
| `exists x in q where p` | `exists(for x in ⟦⟦q⟧⟧ do if ⟦⟦p⟧⟧ then {x} else {})` |
| `not exists x in q where p` | `empty(for x in ⟦⟦q⟧⟧ do if ⟦⟦p⟧⟧ then {x} else {})` |
| `match e \| arms` | the `case` tree of §10.2 |
| `agg(e for x in q where p)` | `agg α (x in (for x in ⟦⟦q⟧⟧ do if ⟦⟦p⟧⟧ then {x} else {}) . ⟦⟦e⟧⟧)` |
| `agg(e for x₁ in q₁, …, xₙ in qₙ where p)` | `agg α (ξ in B . ⟦⟦e⟧⟧[x_i := ξ.x_i])`, `B` the comprehension over the binders selecting `{ x₁: x₁, …, xₙ: xₙ }` |
| `from x₁ in e₁, …, xₙ in eₙ where p group e by k into g where p' select e'` | `for ζ in group (ξ in B) by (let x₁ = ξ.x₁ in … ⟦⟦k⟧⟧) do if ⟦⟦p'⟧⟧ then {⟦⟦e'⟧⟧} else {}`, `B` the comprehension over the generators and `p` selecting `{ x₁: x₁, …, xₙ: xₙ }` |
| after `into`: `g.key` | `ζ.key` |
| after `into`: an aggregate generator `y in g` | `ξ in ζ.rows`, the aggregate's body under `let y = (let x₁ = ξ.x₁ in … ⟦⟦e⟧⟧)` |
| after `into`: `g` anywhere else | `for ξ in ζ.rows do {let x₁ = ξ.x₁ in … ⟦⟦e⟧⟧}` |
| `tag(e)` | `tag ⟦⟦e⟧⟧` |
| `top n by k desc (q)` | `top n by (k desc) (x in ⟦⟦q⟧⟧)`, where `k` is a function of the selected value ([queries.md §14.1](queries.md#141-order-by)), read as a function of the element `x`; a key without direction is `asc` |
| `top n by (k₁ d₁, …, k_j d_j) (q)` | `top n by (k₁ d₁, …, k_j d_j) (x in ⟦⟦q⟧⟧)`, each `k_i` read as a function of `x` as above |
| `order by k`, `order by (k₁, k₂ desc, …)`, `take`, `after` on a final result | `present` (§5.9) over the kernel result |
| `fix a = q₁, b = q₂, … in e` | `fix a = ⟦⟦q₁⟧⟧, b = ⟦⟦q₂⟧⟧, … in ⟦⟦e⟧⟧` |
| `{ l: e, … }` | `{ l: ⟦⟦e⟧⟧, … }` |
| `C { l: e, … }` | `C { l: ⟦⟦e⟧⟧, … }` |
| `C { x with f: v, … }`, `x : T.C` | `C { g: x.g, …, f: ⟦⟦v⟧⟧, … }`, `g` over every field of `C`, key and header included, other than those given |
| `D { x with f: v, … }`, `x : T.S`, `S ≠ {D}` | `D { h: x.h, …, f: ⟦⟦v⟧⟧, … }`, `h` over the key and header fields of `x` only; no field of another constructor is carried, and the own fields of `D` not given are filled by `fill_r` in an update (§10.3) and required elsewhere |
| `if p then a else b` | a match on `Bool`: `case ⟦⟦p⟧⟧ of { False ξ → ⟦⟦b⟧⟧ ; True ξ → ⟦⟦a⟧⟧ }`, written `if ⟦⟦p⟧⟧ then ⟦⟦a⟧⟧ else ⟦⟦b⟧⟧` |
| `try e` | `try ⟦⟦e⟧⟧` |
| a checked operator | the total primitive returning `Option` |
| `key u` | `key ⟦⟦u⟧⟧` |
| `r.key(v)`, `r.key(f: v, g: w)` | `keyof r { k: ⟦⟦v⟧⟧ }` for the single key field `k`, `keyof r { f: ⟦⟦v⟧⟧, g: ⟦⟦w⟧⟧ }` |
| `value(k)`, `k : Key r` | the key fields' value of `k`, the inverse of `keyof r` |
| path `e.f.l` where `f : Key r.S` | `(deref r.S ⟦⟦e⟧⟧.f).l`, repeated along the path |
| backlink `x.<R.f`, `f : Key t.S`, `R` a relation variable, constructor relation, constructor set or named set | `for ξ in ⟦⟦R⟧⟧ do if ξ.f == κ then {ξ} else {}` |
| backlink `x.<R.f`, `f : Option (Key t.S)` | `for ξ in ⟦⟦R⟧⟧ do if ξ.f == Some { value: κ } then {ξ} else {}` |
| an optional reference `e.f : Option (Key r.S)` | no path; matched as an `Option` |
| a nested result `{ l: from … }` | a record field whose value is the kernel comprehension |
| `fn f (x₁: τ₁, …, xₙ: τₙ) -> τ = e` | `λ(x₁ : τ₁). … λ(xₙ : τₙ). ⟦⟦e⟧⟧` |
| a call `f(a₁, …, aₙ)` | `(… (⟦⟦f⟧⟧ ⟦⟦a₁⟧⟧) … ⟦⟦aₙ⟧⟧)`, the arguments applied in order, unfolded by §5.11 |
| `fn v () -> τ = e`, call `v()` | `(λ(ξ : {}). ⟦⟦e⟧⟧) {}` |
| a lambda argument `fn x -> e` | `λ(x : τ). ⟦⟦e⟧⟧` |
| a lambda `fn (x₁: τ₁, …, xₙ: τₙ) -> e` | `λ(x₁ : τ₁). … λ(xₙ : τₙ). ⟦⟦e⟧⟧` |
| `query q (x₁: τ₁, …, xₙ: τₙ) -> τ = e` | the closed term `λ(x₁ : τ₁). … λ(xₙ : τₙ). ⟦⟦e⟧⟧`, applied to the call's arguments in order |
| a virtual computed field `x.f` | the field's defining function applied to `x` ([derived.md](derived.md#5-computed-fields)) |
| a view, stored or not | its defining query ([derived.md](derived.md#1-views)) |

In a backlink, `κ` is `key ⟦⟦x⟧⟧` for a row `x` and `⟦⟦x⟧⟧` for a key
`x`. A relation variable `R` ranges over the constructors of its type
that have the field `f`.

Navigation is sugar: a path is a `deref`, and `deref r.S k` equals the
single element of `for t in r.S do if key t == k then {t} else {}` in a
valid state.

### 10.2 Patterns

A `match e | p₁ -> b₁ | … | pₘ -> bₘ` on a sum `T.S` compiles to

```text
let ξ = ⟦⟦e⟧⟧ in case ξ of { C x_C → M_C }_{C ∈ S}
```

where `M_C` is the chain of the arms that can match constructor `C`,
in their written order. An arm can match `C` when its pattern is
`C …`, an or-pattern with an alternative `C …`, a named set containing
`C`, or, in an ad hoc query only, `_` on the sum. For each such arm
`p_i if g_i -> b_i` (with `g_i = true` when unguarded):

```text
M_C = P(x_C, p_i, if ⟦⟦g_i⟧⟧ then ⟦⟦b_i⟧⟧ else M'_C, M'_C)
```

where `M'_C` is the chain of the remaining arms, and `P(y, p, K, F)`
matches value `y` against pattern `p`, continuing with `K` on success,
in which `p`'s bindings are in scope, and with `F` on failure:

| Pattern `p` | `P(y, p, K, F)` |
| --- | --- |
| binder `z`, or the arm's constructor binder `C z` | `let z = y in K` |
| `_` (scalar) | `K` |
| literal `c` | `if y == c then K else F` |
| field name alone `f` in a record pattern | `let f = y.f in K` |
| record pattern `D { f₁: p₁, …, fₙ: pₙ }` | `P(y.f₁, p₁, P(y.f₂, p₂, … P(y.fₙ, pₙ, K, F) …, F), F)` |
| constructor pattern `D q` on a value-level sum `T'.S'` | `case y of { D ξ → P(ξ, q, K, F) ; E ξ → F for each other E ∈ S' }` |
| or-pattern `p' \| p''` | `P(y, p', K, P(y, p'', K, F))` |

Every binding of a pattern, nested ones included, is in scope in its
guard and its body. The binder of an or-pattern is bound to the row of
whichever alternative matched, which has the or-binder's set type. The
chain for `C` ends in an arm whose pattern cannot fail and that has no
guard, which exhaustiveness guarantees
([typing.md §9.2](typing.md#92-exhaustiveness)); its `F` is never
reached. A `match` on a scalar compiles to the same chain without the
outer `case`, ending at its binder or `_` arm. A `match` on a
scrutinee of type `Key r.S` matches the row the key references:
`let ξ = deref r.S ⟦⟦e⟧⟧ in case ξ of { … }`; its top-level patterns are
constructor patterns, never a binder or `_`. When the scrutinee is not a
reference field's value it may name no row, and the match faults
`NoSuchRow`: it is partial (§3.4). Patterns and guards are
evaluated in arm order; a guard is evaluated only when its pattern
matched (§5.2).

### 10.3 Writes

| Surface ([writes.md](writes.md)) | Kernel |
| --- | --- |
| `insert r.C { l: e, … }` | `insert r {fill_r(C { l: ⟦⟦e⟧⟧, … })}` |
| `insert r from q`, `insert r.C from q` | `insert r (for ξ in ⟦⟦q⟧⟧ do {fill_r(ξ)})` |
| `let x = insert …` in a block | `let! x = (the row or set inserted) in …`, the write `insert r {x}` or `insert r x` joining the block |
| `key (insert r.C { … })` | as `let!` of the row, then `key` of it; only where it is evaluated exactly once per statement, under no binder and in no conditional position ([writes.md §2.2](writes.md#22-what-an-insert-returns)) |
| `update x in R where p set e` | `for x in ⟦⟦R⟧⟧ do if ⟦⟦p⟧⟧ then replace r x ⟦⟦e⟧⟧° else skip` |
| `update x in R where p set match x \| …` | as above, `⟦⟦e⟧⟧` the `case` tree of §10.2 |
| `update o set e` in an upsert branch | `replace r o ⟦⟦e⟧⟧°` |
| `update x set e`, `x` a snapshot row | `replace r x ⟦⟦e⟧⟧°` |
| `update x where p set e` | `if ⟦⟦p⟧⟧ then replace r x ⟦⟦e⟧⟧° else skip` |
| `delete x in R where p` | `delete r (for x in ⟦⟦R⟧⟧ do if ⟦⟦p⟧⟧ then {x} else {})` |
| `delete x`, `x` a snapshot row | `delete r {x}` |
| `delete x where p` | `if ⟦⟦p⟧⟧ then delete r {x} else skip` |
| `match e \| C x -> w \| …` over writes | write-level `case` of §10.2 |
| `skip` | `skip` |
| `(w₁, w₂, …)` | `⟦⟦w₁⟧⟧ ∥ ⟦⟦w₂⟧⟧ ∥ …`, one change set |
| `if c then w₁ else w₂` | the write-level `if ⟦⟦c⟧⟧ then ⟦⟦w₁⟧⟧ else ⟦⟦w₂⟧⟧` |
| `fail e`, `e : Text` | the write term `fail ⟦⟦e⟧⟧`, a rejection, not a fault |
| `insert r.C v unless conflict on K else w` | below |
| `transact` block with lets, writes and `return e` | `transact (let! x₁ = e₁ in … (w₁ ∥ … ∥ wₖ, ⟦⟦e⟧⟧))` |
| `transact` without `return` | result the unit value `()`, the kernel's empty record `{}` |
| `txn t (x₁: τ₁, …, xₙ: τₙ) -> τ = transact …` | `λ(x₁ : τ₁). … λ(xₙ : τₙ). transact …`, applied to the call's arguments in order |

`fill_r(v)` is `v` with every field it omits taken from `r`'s fill
clauses: a default expression, or `fresh g` for a generator `g`. A
field the insert gives is never filled. The filled values are part of
the inserted rows before the change set is formed.

`e°` completes a new value: the row's key, and its header fields where
the surface leaves them out, are taken from the old row, and the
omitted own fields of the new value's constructor are filled by
`fill_r`, whether or not the update retags
([writes.md §4.1](writes.md#41-retag-is-an-update-to-another-constructor)).

An upsert on a key or unique `K` over constructor set `S` with fields
`f̄` (the key fields for `on key`), with source `V` (a query, or `{v}` for one
value); for a unique over expressions, `o.f̄` stands for the values of
its expressions on `o`:

```text
let! ξ_V = for ξ in ⟦⟦V⟧⟧ do {fill_r(ξ)} in
for new in ξ_V do
  let ξ_O = for o in r.S do if o.f̄ == new.f̄ then {o} else {} in
  if empty(ξ_O) then insert r {new} else for old in ξ_O do ⟦⟦w⟧⟧
```

In the else branch `new` is the proposed row, with fills applied,
beside `old`. Bound by `let x`, a single-row upsert binds one row: `new`
when it is inserted; otherwise the row with `old`'s key among the
effects `E_w` of `⟦⟦w⟧⟧` when the branch replaces `old`, and `old` when
the branch leaves it in place. The binding's type is the join of the
types of every value it can denote: `new`, each value `v'` the branch
can write with `old`'s key, and `old` whenever some path through `w`
leaves `old` in place. A `let` on a single-row upsert whose else branch
can delete `old` is a type error; the branch can delete `old` when an
arm deletes it or deletes from a relation whose `cascade` actions,
followed through every relation they delete from, reach `old`'s
relation. The source form binds
the set of the rows it inserted together with the binding, as above,
of each conflicting row. The binding is a `let!` after `ξ_V`, beside the write above:

```text
single row:  let! x = only (U (B(E_w, {old})))
source form: let! x = U (B(E_w, {old}))

U(e) = for new in ξ_V do
         let ξ_O = for o in r.S do if o.f̄ == new.f̄ then {o} else {} in
         if empty(ξ_O) then {new} else for old in ξ_O do e

B(E, d) = A   if A ≠ ∅,   d otherwise,
  where A = { v' | rep(r, old, v') ∈ E or ins(r, v') ∈ E, key_r(v') = key_r(old) }
```

`E_w` is read off the change set, whatever form the branch is written
in; `A` has at most one element, since two rows with one key fail the
statement with `KeyViolation` (§6.3).

In a valid snapshot `ξ_O` has at most one row, since `K` is a key or
unique over `S`, so the single-row set has exactly one element.

The conflict test reads the snapshot
([writes.md §6.2](writes.md#62-meaning)):

- a conflicting row in the snapshot binds `old`, and `w` is evaluated;
- a conflicting row committed by a concurrent transaction after the
  snapshot fails the transaction with a serialization failure (§8.4);
  a concurrent transaction still inserting the same value of `K` is
  waited for, with the same outcome if it commits, and the row is
  inserted if it aborts;
- a source row whose only conflict is a row that another write of the
  same statement inserts binds no `old`, `w` is not evaluated, and the
  two inserted rows share a value of `K`: the statement fails at its
  boundary (§6.6), with `KeyViolation` when `K` is the key and with
  `Violated` carrying the unique as its constraint when `K` is a unique
  (§7.2). Two rows of `ξ_V` with equal `f̄` fail the same way.

### 10.4 Constraints and schema clauses

| Surface ([constraints.md](constraints.md)) | Kernel |
| --- | --- |
| `constraint name = q` | `⟦⟦q⟧⟧` |
| `check name on R (p)`, `R` a constructor relation or set of `r` | `for x in ⟦⟦R⟧⟧ do if ⟦⟦p⟧⟧[self := x, f := x.f] then {} else {x}`, each field name `f` and `self` bound to the row |
| `type N = new T where p` | the domain constraint of §7.2 |
| `key k` of `r` | the key constraint of §7.2 |
| `unique (f̄) in S` | the unique constraint of §7.2 |
| a field `f : Key r.S` or `f : Option (Key r.S)` | the reference constraint of §7.2, with its delete action in §6.4 |
| `on delete restrict`, `cascade`, `retag D { … }`, `clear` | the action of §6.4; `clear` only on `Option (Key r.S)` |
| `inserted(R)`, `deleted(R)`, `prior(R)` | `inserted r.S`, `deleted r.S`, `prior r.S` |

### 10.5 Migrations

| Surface ([evolution.md](evolution.md)) | Kernel |
| --- | --- |
| a migration file `n` | the operators `op_k … op_l` of §9.3, applied as one statement, the composite of their maps (§9.5) |
| `split … by match \| arms` | `φ` = the `case` tree of §10.2 over the old row |
| `merge … into C { discriminator: …, … }` | `φ(C_i⟨ḡ⟩) = C⟨discriminator: C_i, ḡ⟩` |
| `add constructor … arm q: C x -> e` | the new `case` arm `C x → ⟦⟦e⟧⟧` in each named query |
| `revert n fill T.C.f = e …` | the inverses of migration `n`'s operators, in reverse order; an inverse that adds field `T.C.f` takes `e` as its fill (§9.4) |

Rulings: query/construct, lang/nav/paths, lang/nav/backlinks, lang/nav/joins, lang/nav/backlink-card/collection-typed, lang/nav/path-option/option-typed, lang/patterns/guards, lang/patterns/guard-cover/ignore-guarded, lang/patterns/literals, lang/patterns/nested, lang/patterns/or-patterns, lang/patterns/or-binder/ctor-set, lang/patterns/record, lang/wildcard/verbosity/adhoc-wildcard, lang/wildcard/verbosity/set-patterns, lang/nullary-fn, lang/params/declared, lang/registered/named, lang/abstraction/higher-order, syntax/aggregate/comprehension, write/insert/ctor, write/insert/value, write/update/ctor-binder, write/update/match-set, write/nested/replace, write/nested/rebuild, write/upsert/unless-conflict, write/upsert/cross-ctor/match-else, write/transact/block, write/transact/registered-txn, surface/constraint/check, surface/constraint/denial, derived/defaults/on-relation, derived/materialize/storage-choice, derived/computed/virtual, model/prelude-bool, surface/query-scoped, evolution/split-routing/match-route, constraint/scope/new-ctor/arm-in-add, spec/lambda/params/curried, spec/top/operand/comprehension, spec/writes/upsert-binding/change-set, spec/writes/nested-insert/once, spec/evolution/migration-statement/one-statement, spec/evolution/revert-fill/per-field, spec/writes/bound-row/snapshot-row, spec/writes/where-alone/conditional, spec/writes/upsert-delete/through-actions
