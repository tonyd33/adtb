# Cheney, Lindley, Wadler: language-integrated query

kind: paper
home: https://homepages.inf.ed.ac.uk/slindley/papers/practical-theory-of-linq.pdf

## Query language

- "A practical theory of language-integrated query", ICFP 2013, DOI 10.1145/2500365.2500586; theoretical model T-LINQ, implementation P-LINQ, based on quotation and normalisation of quoted terms. [S 2026-10-25] https://dl.acm.org/doi/10.1145/2500365.2500586
- Normalisation turns any closed, nonrecursive, effect-free quoted query of flat type into a single flat SQL query, allowing lambdas and abstraction over predicates. [U]
- Query shredding (SIGMOD 2014) evaluates nested multiset results as a fixed number of flat queries. [U]
- Links (Cooper, Lindley, Wadler, Yallop, FMCO 2006) types queries with row-polymorphic records and variants. [U]
