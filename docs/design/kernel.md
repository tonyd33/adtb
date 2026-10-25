# Kernel terms

`adtb-lang` provides typed kernel terms to three consumers: the engine
`adtb`, the reference interpreter `adtb-reference`
([oracle.md](oracle.md)), and code generation tools for host languages.
The kernel calculus itself is
[../spec/semantics.md §2](../spec/semantics.md#2-kernel-syntax).

## 1. What a consumer receives

The engine and the reference interpreter both consume typed kernel
terms and share nothing else.

Postconditions on a term `adtb-lang` hands to the planner, as
[../spec/semantics.md §5.11](../spec/semantics.md#511-normalisation)
states them:

- It is closed, and its free variables have first-order types.
- It is normalised: it contains no λ and no application, and a `let`
  remains only where its bound expression is partial.
- Every call of a generic function is instantiated at the type
  arguments [../spec/typing.md §2.2](../spec/typing.md#22-checked-and-synthesized-forms)
  solves.

TODO: whether `adtb-lang` or the engine performs normalisation is undecided.

TODO: the remaining invariants of a kernel term (constructor sets listed at each source, fills explicit) and its representation are undecided.

## 2. Code generators

Code generation tools for host languages depend on `adtb-lang` alone.
They compute type, constructor and relation variable ids and call
ordinals as [../spec/wire.md §4.3](../spec/wire.md#43-records-and-sums)
and [§5.1](../spec/wire.md#51-call) assign them, so generated code
agrees with every database built from the same log.

TODO: the API by which `adtb-lang` gives a code generator these ids and ordinals is undecided.

