---
title: Links and T-LINQ
---
Links (Cooper, Lindley, Wadler, Yallop, FMCO 2006) types database
queries with row-polymorphic records and variants and compiles a
comprehension to one SQL query. T-LINQ (Cheney, Lindley, Wadler, ICFP
2013) quotes queries in the host language, allows lambdas and
abstraction over predicates, and normalises any closed, nonrecursive,
effect-free quoted query of flat type to a single flat query. Query
shredding (SIGMOD 2014) extends this to nested multiset results.
https://homepages.inf.ed.ac.uk/slindley/papers/practical-theory-of-linq.pdf
