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
to right; the members of a family indexed by the bindings of a set are
gathered:

```text
seq (o₁, …, oₙ)       =  (v₁, …, vₙ)    if every o_i = v_i
                      =  o_j            for the least j with o_j an abort, otherwise
gather (o_i)_{i ∈ I}  =  (v_i)_{i ∈ I}  if every o_i = v_i
                      =  o_j            for some j with o_j an abort, otherwise
```

Within a binding the first abort reached is the outcome. When several
bindings abort, the outcome is any one of their aborts, so a `try` over
several faulting bindings returns `Err` of the fault of any one of them.

### 1.2 Kernel and surface

The kernel (§2) is what the semantics is given on. Every surface form
is a rewrite into it (§10). The rewrite is total on well-typed surface
programs and preserves typing: a surface program of type `T` desugars to
a kernel term of type `T`.

### 1.3 Reference interpreter

Every equation in this part reads as a program over finite sets, and
the reference interpreter ([implementation.md](implementation.md))
evaluates kernel terms by these equations. An implementation conforms
when, for every program and every state, its outcome is one the
equations allow. The equations fix the outcome except in four places,
each stated where it arises:

| Freedom | Section | Conformance compares |
| --- | --- | --- |
| Which abort is reported when several bindings abort, and which fault a `try` over them returns | §1.1 | that both outcomes are aborts; under `try`, that the fault is one some binding raises |
| The values of generated keys and generated clock fields | §4.3 | results up to a bijection on generated values |
| The fold order of an aggregate whose operator is not associative and commutative | §5.7 | results within a bound that grows with input size |
| The step at which a statement exceeds its limits, an unbounded fixpoint included | §5.10 | that both are rejected for a limit |

Rulings: project/semantics/denotational, project/semantics/kernel/core, project/semantics/covers/queries, project/semantics/covers/writes, project/semantics/covers/constraints, project/semantics/covers/isolation, project/semantics/covers/evolution, project/oracle/rust-reference, project/oracle/shared-front/typed-input, project/oracle/float/tolerance

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
registered query; every function is normalised away before planning
(§5.11). A result sum anonymous to a query is a
declared sum local to that query's signature. `Bool` is the prelude
sum `False | True`; `Option` and `Result` are prelude sums
([model.md §5.2](model.md#52-library-sums)). `Set⁺ τ` is a kernel type
with no surface spelling; it types the rows of a group (§5.8).

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
  a delete action's `retag` derives (§6.4); never under a λ;
- `only e` occurs only in the binding of a single-row upsert (§10.3),
  where `e` has exactly one element;
- `inserted`, `deleted` and `prior` occur only in the body of a
  constraint;
- a `case` names each constructor of its scrutinee's constructor set
  exactly once.

Rulings: project/semantics/kernel/core, query/ctor-relations, lang/family/comprehension, lang/recursion/fix, lang/abstraction/higher-order, lang/abstraction/ho-recursion/first-order-recursion, write/primitive/delta, write/surface/txdata, derived/transition/change-visible, types/fault-forms/try, lang/group/into, lang/nested-order/top-n, lang/group/ctor-key/tag-type, model/typed-key, model/user-params, model/prelude-bool

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
  [prelude.md](prelude.md));
- an aggregate whose combining operator or final map is partial on its
  type (`sum` over an integer type);
- `top` whose count is not a nonnegative literal;
- a `fix` that is not bounded (§5.10), which may not terminate;
- a `deref` of a key that is not a reference field's value;
- any `deref`, and every path through a reference, in a bundle
  declaring `hide new constructors` (§9.6);
- an application of a function whose body is not total.

`try e` is total whatever `e` is. A total expression terminates and
denotes a value in every environment that its type admits; it never
faults. Limits (§1.1) are outside totality.

Rulings: lang/binder/ctor-row, lang/patterns/or-binder/ctor-set, model/ref-subtyping/ctor-set-subsumption, query/compile-time, lang/wildcard/scalars-only, lang/empty-agg/nonempty, types/join/lub-exact-fields, types/sub-records/exact, types/eq-types/all-first-order, lang/collection/set-agg/over-binders, lang/recursion/negation/stratified, types/fault-forms/checked

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

Heaps, pages and indexes are representations of `D`; no program
observes them.

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
  distinct from every key of `r` in the database's current state
  (including rows the statement's version does not see, §9.6), and
  distinct from every value already issued
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

Rulings: model/sets, model/nominal, model/one-ctor-sum, model/record-is-ctor, sum/closed, sum/disjoint-total, sum/relation, model/one-key, model/declared-key, model/no-collections, model/sum-header, model/set-of-type, lang/results/nested, types/eq/leibniz, engine/guarantees-only, engine/catalog-relations, types/now/param-only, types/id-type/key-type, types/keygen/promise/distinct

## 5. Queries

`⟦e⟧ ρ σ ∈ O(V)`. `D` below is the snapshot of `σ`.

### 5.1 Scalar forms

```text
⟦x⟧ ρ                    = ρ(x)
⟦c⟧ ρ                    = c
⟦op(e₁,…,eₙ)⟧ ρ          = (v₁,…,vₙ) ⇐ seq(⟦e_i⟧ ρ) ; op(v₁,…,vₙ)
⟦{ l_i: e_i }⟧ ρ         = (v_i) ⇐ seq(⟦e_i⟧ ρ) ; { l_i: v_i }
⟦C { l_i: e_i }⟧ ρ       = (v_i) ⇐ seq(⟦e_i⟧ ρ) ; C⟨v_i⟩
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

The operands of `op`, a record and a constructor value are evaluated
left to right, each before the operation. A primitive `op` is a
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
right. An implementation may evaluate a subterm where these equations
do not evaluate it, or earlier, only when the subterm is total (§3.4).
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

Executing the union as one probe of a union index, routed by
constructor, is an implementation of this equation
([engine.md](engine.md)).

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
present(A, k̄, n, c) = the first n elements of { v ∈ A | c ≺ v }, listed by ≺
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
([catalog.md §8.3](catalog.md#83-limits-and-deadlines)). An implementation
may compute `S̄*` by semi-naive iteration or any other method that
yields the same sets.

A `fix` is bounded when no body applies a constructing primitive, an
aggregate or `fresh` to a value derived from a relation of its own
stratum, and no body places an element of such a relation inside a
constructed record or constructor value. A constructing primitive is
one whose result may be a value that is none of its arguments:
arithmetic, text and byte building, conversions, time arithmetic.
Comparisons, `==`, `!=`, boolean operations and other primitives
returning `Bool` or one of their arguments are not constructing, so a
reachability `fix` that compares keys is bounded. Every value a bounded `fix`
adds is built from finitely many values present before it runs, so its
least fixpoint is finite and it is total. A `fix` that is not bounded
is partial (§3.4).

Stratification. The relations bound by one `fix` form one stratum. The
positive positions of each `X_j` in a body `e_i`, and the positive
conditions, are defined by:

- `e_i` itself is positive;
- if `e' ∪ e''` is positive, so are both operands;
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
types. Before planning, it is normalised by these rules, where `e'`
total means `e'` is total (§3.4):

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
§5.3, aborts included. The planner sees only normalised terms.

Rulings: query/union-all, query/ctor-relations, query/construct, query/native, model/dup-arms/dedupe, lang/collection/sets, lang/collection/set-agg/over-binders, lang/empty-agg/nonempty, query/agg-scope/whole, lang/group/into, lang/group/ctor-key/tag-type, lang/nested-order/top-n, lang/order/result-only, lang/keyset, lang/recursion/fix, lang/recursion/negation/stratified, lang/abstraction/higher-order, lang/abstraction/ho-recursion/first-order-recursion, lang/outer/nested, lang/results/nested, types/fault/abort, types/fault/reorder/ordered-guards, types/fault-forms/checked, types/fault-forms/try, types/int-overflow/trap, project/oracle/float/tolerance, derived/transition/change-visible

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
⟦w₁ ∥ w₂⟧ ρ                  = (E₁, E₂) ⇐ seq(⟦w₁⟧ ρ, ⟦w₂⟧ ρ) ; E₁ ∪ E₂
⟦for x in e do w⟧ ρ          = A ⇐ ⟦e⟧ ρ ; (E_v) ⇐ gather(⟦w⟧ ρ[x ↦ v])_{v ∈ A} ; ⋃ E_v
⟦if e then w₁ else w₂⟧ ρ     as §5.2
⟦case e of { C x → w_C }⟧ ρ  as §5.2
⟦let x = e in w⟧ ρ           = v ⇐ ⟦e⟧ ρ ; ⟦w⟧ ρ[x ↦ v]
⟦fail e⟧ ρ                   = t ⇐ ⟦e⟧ ρ ; ⊗UserError(t)
```

Every read in a write term reads the snapshot `D`; no write sees
another write's effects except through a `let!` binding (§6.6). The
order of `∥` operands has no effect on the change set. `fail` rejects
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
  reported at the statement boundary (§6.6).

### 6.4 Delete actions

For a set of effects `E`, let `Δ_E` be its change set (§6.3) and
`D_E = apply(D, Δ_E)` (§6.5). A key `k` of `r` is gone in `E` when it is
a key of a row of `D(r)` and of no row of `D_E(r)`. A reference value
`u.f` names `k` when `f : Key r.S` and `u.f = k`, or
`f : Option (Key r.S)` and `u.f = Some⟨k⟩`. The actions derive effects:

```text
A(E) = for every reference field f of a constructor C of T_r' into r,
       every row u ∈ D_E(r'.C) whose u.f names a key gone in E, with
       key_r'(u) not gone in E:
         cascade        kill(r', key_r'(u))
         retag D {…}    rep(r', u₀, fill_r'(D⟨key and header fields of u, payload⟩))
         clear          rep(r', u₀, u with f = None)
         restrict       nothing
       where u₀ is the row of D(r') with u's key; retag and clear derive
       nothing for a u with no such row
```

The statement's effects are the least `E*` with `E ⊆ E*` and
`A(E*) ⊆ E*`, reached by iterating `E_{i+1} = E_i ∪ A(E_i)` from the
written effects. Each derived effect names a row of `D` or of the
written effects, and each draws its fills from fixed instances of `γ`,
so the iteration ends.

Consequences, matching [writes.md §7.2](writes.md#72-when-an-action-runs):

- a row deleted explicitly or by `cascade` is deleted, even when a
  `retag` or `clear` action also reaches it: `kill` removes every
  inserted row of its key;
- an explicit update and a `retag` or `clear` action that give one row
  different new values leave two rows with its key, a `KeyViolation`;
- a row inserted by the statement that references a gone key is reached
  by `cascade` only; under `retag` or `clear` it remains and fails the
  reference constraint;
- an update or a retag of the target never makes its key gone and fires
  no action;
- `restrict` derives nothing; the reference constraint (§7.2) rejects
  the statement if a referrer of a gone key remains.

`fill_r'` of a `retag` action's value fills the fields of `D` the payload
omits, as an insert into `r'.D` does, drawing from `γ`.

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

The outcome of `transact b` in context `σ = (D, Σ, v, γ)`:

```text
run(s, σ) =
  (E, res) ⇐ ⟦b⟧ ∅ σ ;
  E*  = the closure of E under the delete actions (§6.4) ;
  Δ*  = normalise(change set of E*) ;  D* = apply(D, Δ*) ;
  ⊗NotOpen         if some v ∈ Δ*⁺(r.C) with C not open for writes in stage(Σ)
                   has no row of its key in D(r.C): an insert or a retag into C;
                   or if some v ∈ Δ*⁺(r.C) holds in a field a value of a
                   value-level constructor E not open for writes, and the row
                   of its key in D(r), if any, holds no value of E in that field
  ⊗KeyViolation    if D*(r) holds two different rows with one key, for some r
  viol = { (c, R_c) | c ∈ cons(Σ), R_c ⇐ ⟦c⟧(D, Δ*, D*), R_c ≠ ∅ } ;
  ⊗Violated(c, R_c) if viol ≠ ∅, for the (c, R_c) ∈ viol whose c has the smallest name
  (res, Δ*, D*)    otherwise
```

An update that keeps a row in a deprecated constructor, or keeps a
field's value in a deprecated value-level constructor, is not refused.
The rejection carries one violated constraint and its report rows; the
caller sees the rows its grants allow
([constraints.md §7](constraints.md#7-checking-and-violation-reports)).

An abort anywhere in this computation is the statement's outcome, and
no part of its change set is applied. The key and constraint tests are
made at the statement boundary, on `D*`. A statement that succeeds
replaces `D` by `D*` for the statements after it in its transaction.
`D` is the snapshot: a test that would meet a row committed by another
transaction after the snapshot is not decided here; it ends the
transaction with a serialization failure (§8.4). `NotOpen`,
`KeyViolation`, `Violated`, `KeyChanged` and `UserError` are
constructors of `RejectReason` ([catalog.md](catalog.md#4-error-sums)).

Rulings: write/primitive/delta, write/surface/txdata, write/unit/multi, write/retag/assign, write/retag/old/in-scope, write/transact/block, write/insert/returns/rows, write/bulk/from-query, write/delete-action/cascade, write/delete-action/restrict, write/delete-action/retag-referrer, fk/retag/recheck, model/deferred-check, constraint/timing/multi-assign, types/keygen/promise/distinct

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
([constraints.md §5](constraints.md#5-admission)). An incremental check
`δc` reads `Δ*`, probes of `D` and `D*`, and returns a set. It is
correct when, for every valid `D` and every change set:

```text
δc(D, Δ*, D*) = ⟦c⟧(D, Δ*, D*)
```

The implementation must use only correct checks. Constraints whose
query reads no constructor relation that `Δ*` touches return the same
set as in `D`, which is empty, and need not be evaluated.

Rulings: constraint/form/empty, constraint/assertion, constraint/timing/multi-assign, constraint/enforce/delta, constraint/admit/bounded, derived/transition/change-visible, surface/constraint/check, surface/constraint/denial, constraint/scope/ctor-set, fk/to-sum, fk/to-ctor, fk/to-subset, fk/retag/recheck, sum/disjoint-total

## 8. Isolation

### 8.1 Transactions and histories

A transaction is a sequence of statements `s₁ … s_m`. A transact block
or a registered transaction is a transaction of one statement; an
interactive transaction is one statement per call
([transactions.md §2](transactions.md#2-statements)). A migration
applies as a transaction too, each of its operators one statement
(§9.3, §9.5). A history is the set of
transactions the database has run, each committed or aborted, with the
state each committed transaction installed. Only committed
transactions change the state.

### 8.2 Items, reads and writes

The items are the rows `(r, k)` of each relation variable, named by key,
whatever their constructor. A committed transaction whose change set
holds `k` in `Δ*⁺(r.C)` or `Δ*⁻(r.C)` writes `(r, k)` and installs a
version of it: a row of some `r.C`, or its absence. A retag installs one
version, in the new constructor relation.

A statement reads constructor relations through predicates. Its
denotation depends on `D(r.C)` only through sets `{ v ∈ D(r.C) | P(v) }`
for predicates `P` the implementation tracks; a read of `r.S` is a
predicate read of every `r.C` with `C ∈ S`. Constraint checks are reads
of the statement that triggers them. The schema version is an item too:
every statement reads the version it runs at, and applying a migration
writes it, so a writer at version `n - 1` is serialized against the
migration that produces `n`. A write `T'` changes the matches
of a predicate read of `r.C` by `T` when a version `T'` installs, or the
one it replaces, is a row of `r.C` that satisfies `P`.

### 8.3 Dependency graph

The dependency graph of a history has the committed transactions as
nodes and an edge `T → T'` when:

| Edge | Condition |
| --- | --- |
| write-write | `T'` installs the next version of an item after a version `T` installed |
| write-read | `T'` reads a version `T` installed, directly or through a predicate |
| read-write | `T'` installs the next version of an item, or changes the matches of a predicate, after the version or state `T` read |

### 8.4 Guarantee

Every history the database produces has an acyclic dependency graph
over its committed transactions. Equivalently, there is a total order
`T₁ < T₂ < … < T_n` of the committed transactions such that running
each statement of each `T_i` by `run` (§6.6), or by the translated run
of §9.6 for a statement at an older version, in that order, from the
initial state, with the same supplies `γ`, gives every statement the
outcome the database returned to it, and leaves the state the database
holds. This is the only isolation level. A transaction that would make
the graph cyclic does not commit; it fails with a serialization failure
and may be retried ([transactions.md](transactions.md#55-retry)).

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

Each case is a read-write edge whose cycle the guarantee forbids.
Duplicate keys among a statement's own inserted rows are
`KeyViolation` (§6.6).

Two consequences: concurrent transactions cannot jointly violate a
constraint that each satisfies alone, since each statement's check is a
read; and a retag is ordered against every concurrent statement that
references, reads or retags the same row.

Rulings: engine/serializable-only, engine/ssi, engine/retag-fk/by-isolation, mvcc/ctor-tagged, constraint/concurrent/serial-checks, write/transact/interactive, write/transact/registered-txn, engine/transactional-ddl

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
fwd_op : DB(Σ) ⇀ DB(Σ')                   the data map applied with the operator
bwd_op : DB(Σ') → DB(Σ)                   the inverse map: Σ' state seen at Σ
put_op : DB(Σ') × DB(Σ) ⇀ DB(Σ')          a Σ write carried into a Σ' state
π_op   : DB(Σ) → DB(Σ)                    the declared loss; the identity unless declared
```

`fwd_op` is partial: an operator whose preconditions fail on the
current state is refused (removing a populated constructor, adding a
constraint the data violates). The laws:

```text
(L1)  bwd(fwd(D)) = π(D)              for D ∈ dom fwd
(L2)  bwd(put(D', D)) = D             where put(D', D) is defined
(L3)  put(D', bwd(D')) = D'
(L4)  π ∘ π = π
(L5)  c ∈ cons(Σ) is carried to c' ∈ cons(Σ'), and D valid ⟹ fwd(D) valid
```

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

where `φ(v) ⊕ v'` keeps from `v'` the fields of `Σ'` that `φ⁻¹` drops.
When `D'` has no row with `v`'s key (a row inserted at `Σ`), those
fields take the operator's fill: the fill of an `add field`, the fill
of a field moved up to the header.
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
| add or drop a unique, an index, a constraint, a check, a computed field | identity; adding a unique, constraint or check requires the data to satisfy it; a dropped unique, constraint or check stays enforced while its drop is pending | identity | none |
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
to `Key r.(C₁ | … | C_k)`. Removing a populated constructor is a merge
into a surviving constructor followed by a remove; remove itself never
deletes rows. The data map of each operator of a migration is one
statement of the migration's transaction (§9.5), with change set
`diff(D, fwd(D))`, and the constraints of `Σ'` are checked on its
result. `disable` is the inverse of `enable`, and `deprecate` applies
only to an enabled constructor. Retiring a bundle is a command, not an
operator, and changes no schema ([applications.md](applications.md)).

A script changes data, not types: `Σ' = Σ`, and `fwd` is the state
after its change set. Translation across a script is the identity, so
a reader pinned before it reads the data the script wrote, through the
maps of the other operators. A script is never refused or delayed for
pinned bundles. It satisfies none of L1 to L3 and has no inverse;
`revert` of a migration containing one is refused.

### 9.4 Undo

`revert n` appends, in reverse order, the inverses of the operators of
migration `n`. The inverse of an `arm` clause of `add constructor`
removes that arm from its query. The log keeps both; versions are never reused. A revert
whose inverses are refused on the current state is refused.

### 9.5 The log as one history

A migration is a transaction (§8.1): its operators apply together as
one schema change, producing the version its number names. Operators
that change definitions (functions, constraints) and lifecycle stages
are entries of the same log as operators that change data, and are
versioned by it.

### 9.6 Version translation

For versions `n ≤ m`, let

```text
B_{n←m} = bwd_{op_{n+1}} ∘ … ∘ bwd_{op_m}     : DB(Σ_m) → DB(Σ_n)
P_{n←m}(D_m, D_n) = put_{op_m}(D_m, put_{op_{m-1}}(… , D_n))   composed likewise
```

A query `q` registered at version `n` and called when the database is
at version `m` means, with the schema functions, view definitions,
named-set memberships and types of `Σ_n`:

```text
⟦q⟧^{n}(D_m) = ⟦q⟧_{Σ_n}(B_{n←m}(D_m))
```

An implementation translates `q` once into a query `q'` over `Σ_m`; the
translation is correct when `⟦q'⟧_{Σ_m}(D_m) = ⟦q⟧^{n}(D_m)` for every
valid `D_m`.

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
when `P` is defined on every state its writes can produce. Every live
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

Rulings: evolution/smo, evolution/add, evolution/remove, evolution/rename, evolution/split, evolution/merge, evolution/move-field, evolution/nest, evolution/merge-tag/discriminator, evolution/split-routing/match-route, evolution/hoist/default, evolution/remove-rows/restrict, evolution/remove-rows/merge-first, evolution/revert, evolution/scripted, evolution/in-log, evolution/log-ops, evolution/old-readers/readers-first, model/param-evolution/live, constraint/evolve/carry, constraint/evolve/icmo, constraint/evolve/policy/check, constraint/evolve/unmapped/refuse, derived/defaults/backfill/separate, types/text-eq/unassigned/pin-unicode, surface/source/op-log, surface/version/counter, surface/version/history/linear, surface/version-serving/translate, surface/version-serving/untranslatable/translate-at-apply, engine/transactional-ddl, surface/registration/order/pinned

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
| `top n by k desc (q)` | `top n by (k desc) (x in ⟦⟦q⟧⟧)`, `x` the binder `k` is written over; a key without direction is `asc` |
| `top n by (k₁ d₁, …, k_j d_j) (q)` | `top n by (k₁ d₁, …, k_j d_j) (x in ⟦⟦q⟧⟧)` |
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
| `fn f (x̄: τ̄) -> τ = e`, call `f(ā)` | `(λ(x̄ : τ̄). ⟦⟦e⟧⟧) ⟦⟦ā⟧⟧`, unfolded by §5.11 |
| `fn v () -> τ = e`, call `v()` | `(λ(ξ : {}). ⟦⟦e⟧⟧) {}` |
| a lambda argument `fn x -> e` | `λ(x : τ). ⟦⟦e⟧⟧` |
| `query q (x̄: τ̄) -> τ = e` | the closed term `λ(x̄ : τ̄). ⟦⟦e⟧⟧`, applied to the call's arguments |
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
| `key (insert r.C { … })` | as `let!` of the row, then `key` of it |
| `update x in R where p set e` | `for x in ⟦⟦R⟧⟧ do if ⟦⟦p⟧⟧ then replace r x ⟦⟦e⟧⟧° else skip` |
| `update x in R where p set match x \| …` | as above, `⟦⟦e⟧⟧` the `case` tree of §10.2 |
| `update o set e` in an upsert branch | `replace r o ⟦⟦e⟧⟧°` |
| `delete x in R where p` | `delete r (for x in ⟦⟦R⟧⟧ do if ⟦⟦p⟧⟧ then {x} else {})` |
| `match e \| C x -> w \| …` over writes | write-level `case` of §10.2 |
| `skip` | `skip` |
| `(w₁, w₂, …)` | `⟦⟦w₁⟧⟧ ∥ ⟦⟦w₂⟧⟧ ∥ …`, one change set |
| `if c then w₁ else w₂` | the write-level `if ⟦⟦c⟧⟧ then ⟦⟦w₁⟧⟧ else ⟦⟦w₂⟧⟧` |
| `fail e`, `e : Text` | the write term `fail ⟦⟦e⟧⟧`, a rejection, not a fault |
| `insert r.C v unless conflict on K else w` | below |
| `transact` block with lets, writes and `return e` | `transact (let! x₁ = e₁ in … (w₁ ∥ … ∥ wₖ, ⟦⟦e⟧⟧))` |
| `transact` without `return` | result the unit value `()`, the kernel's empty record `{}` |
| `txn t (x̄: τ̄) -> τ = transact …` | `λ(x̄ : τ̄). transact …`, applied to the call's arguments |

`fill_r(v)` is `v` with every field it omits taken from `r`'s fill
clauses: a default expression, or `fresh g` for a generator `g`. A
field the insert gives is never filled. The filled values are part of
the inserted rows before the change set is formed.

`e°` completes a new value: the row's key, and its header fields where
the surface leaves them out, are taken from the old row, and a retag
fills the omitted own fields of the new constructor by `fill_r`
([writes.md §4.1](writes.md#41-retag-is-an-update-to-another-constructor)).

An upsert on a key or unique `K` over constructor set `S` with fields
`f̄` (the key fields for `on key`), with source `V` (a query, or `{v}` for one
value):

```text
let! ξ_V = for ξ in ⟦⟦V⟧⟧ do {fill_r(ξ)} in
for new in ξ_V do
  let ξ_O = for o in r.S do if o.f̄ == new.f̄ then {o} else {} in
  if empty(ξ_O) then insert r {new} else for old in ξ_O do ⟦⟦w⟧⟧
```

In the else branch `new` is the proposed row, with fills applied,
beside `old`. Bound by `let x`, a single-row upsert binds one row: `new`
when it is inserted; otherwise `v'` when the effects of `⟦⟦w⟧⟧` hold
`rep(r, old, v')`, and `old` when they do not. The source form binds
the set of the rows it inserted together with the rows its else branch
wrote. In the kernel, the binding is a `let!` after `ξ_V`, beside the
write above:

```text
single row:  let! x = only (U (case W(⟦⟦w⟧⟧) of { None ξ → {old} ; Some ξ → {ξ.value} }))
source form: let! x = U (case W(⟦⟦w⟧⟧) of { None ξ → {} ; Some ξ → {ξ.value} })

U(e) = for new in ξ_V do
         let ξ_O = for o in r.S do if o.f̄ == new.f̄ then {o} else {} in
         if empty(ξ_O) then {new} else for old in ξ_O do e
```

`W(w)` is `Some` of the new value `w` gives the conflicting row, or
`None` when it gives none:

| Write `w` | `W(w)` |
| --- | --- |
| `replace r y e`, `y` the conflicting row or a `case` binder of it | `Some { value: e }` |
| `if c then w₁ else w₂` | `if c then W(w₁) else W(w₂)` |
| `case e of { C y → w_C }` | `case e of { C y → W(w_C) }` |
| `let y = e in w'` | `let y = e in W(w')` |
| `w₁ ∥ w₂` | `case W(w₁) of { None ξ → W(w₂) ; Some ξ → ξ }` |
| any other write | `None {}` |

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
| a migration file `n` | the operators `op_k … op_l` of §9.3, applied as one transaction |
| `split … by match \| arms` | `φ` = the `case` tree of §10.2 over the old row |
| `merge … into C { discriminator: …, … }` | `φ(C_i⟨ḡ⟩) = C⟨discriminator: C_i, ḡ⟩` |
| `add constructor … arm q: C x -> e` | the new `case` arm `C x → ⟦⟦e⟧⟧` in each named query |
| `revert n` | the inverses of migration `n`'s operators, in reverse order |

Rulings: query/construct, lang/nav/paths, lang/nav/backlinks, lang/nav/joins, lang/nav/backlink-card/collection-typed, lang/nav/path-option/option-typed, lang/patterns/guards, lang/patterns/guard-cover/ignore-guarded, lang/patterns/literals, lang/patterns/nested, lang/patterns/or-patterns, lang/patterns/or-binder/ctor-set, lang/patterns/record, lang/wildcard/verbosity/adhoc-wildcard, lang/wildcard/verbosity/set-patterns, lang/nullary-fn, lang/params/declared, lang/registered/named, lang/abstraction/higher-order, syntax/aggregate/comprehension, write/insert/ctor, write/insert/value, write/update/ctor-binder, write/update/match-set, write/nested/replace, write/nested/rebuild, write/upsert/unless-conflict, write/upsert/cross-ctor/match-else, write/transact/block, write/transact/registered-txn, surface/constraint/check, surface/constraint/denial, derived/defaults/on-relation, derived/materialize/storage-choice, derived/computed/virtual, model/prelude-bool, surface/query-scoped, evolution/split-routing/match-route, constraint/scope/new-ctor/arm-in-add
