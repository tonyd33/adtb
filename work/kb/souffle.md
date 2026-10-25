# Soufflé

kind: language
home: https://souffle-lang.github.io

## Types

- Record types `.type R = [f1: T1, ..., fk: Tk]`, may be recursive, terminated by `nil`. [V 2026-10-25] https://souffle-lang.github.io/types
- ADTs: branches `$Branch(args)`, may be recursive, need a non-recursive branch to terminate. [V 2026-10-25] https://souffle-lang.github.io/types
- `.type X <: number` declares a subtype of a primitive with subset semantics; `.type X = number` is an equivalence (alias). [V 2026-10-25] https://souffle-lang.github.io/types
- Records are interned in a record table and represented by an ordinal. [U]

## Recursion

- Stratified negation: negation not allowed through recursion; unstratifiable programs rejected. [U] (search returned only secondary sources)

## ADTs

- `.type E = A{..} | B{..}`, built with `$A(..)`, matched by unification; no documented exhaustiveness. [V 2026-10-25] https://souffle-lang.github.io/types
- Records looked up by full record or id; partial-record queries unsupported. [S 2026-10-25] https://www.philipzucker.com/datalog-book/adt.html
