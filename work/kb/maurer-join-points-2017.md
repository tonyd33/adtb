# Maurer, Downen, Ariola, Peyton Jones: Compiling without Continuations (PLDI 2017)

kind: paper
home: https://www.microsoft.com/en-us/research/wp-content/uploads/2016/11/join-points-pldi17.pdf

## Join points

- Commuting conversions such as case-of-case duplicate the outer case's alternatives into each inner branch; the duplication can be exponential when cases nest. Binding the shared continuation with an ordinary `let` avoids it but the binding may then be compiled as a heap-allocated closure. [V 2026-10-29] §2 of the PDF above
- System FJ extends GHC Core with two forms: `join j a x = u in e` (a possibly recursive join point with type and value parameters) and `jump j ϕ e τ`, a saturated jump that also takes the result type. [V 2026-10-29] §3, Fig. 1
- A join point is typed `∀a. σ → ∀r. r`; a jump instantiates `r`, so a jump can stand at any type. The typing judgement keeps join points in a separate environment Δ, reset to empty for every subterm evaluated in an unknown context (a lambda body, a function argument), so jumps occur only in tail contexts. [V 2026-10-29] §3, Fig. 2
- Tail contexts are the hole, case alternatives, let bodies and join bodies/scopes; a jump discards its evaluation context. [V 2026-10-29] §3
- Transformations (case-of-case, float) push an evaluation context into the join point's body instead of copying it to each jump, so join points survive later rewrites. [V 2026-10-29] §4
