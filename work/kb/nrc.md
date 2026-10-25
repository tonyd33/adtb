# Nested relational calculus

kind: paper
home: https://doi.org/10.1016/0304-3975(95)00024-Q

## Model

- Buneman, Naqvi, Tannen, Wong, "Principles of programming with complex objects and collection types", TCS 149(1):3-48, 1995: record and set types combine freely; extends to bags and lists. [S 2026-10-25] https://www.research.ed.ac.uk/en/publications/principles-of-programming-with-complex-objects-and-collection-typ/
- Conservative extension: NRC queries from flat input to flat output are expressible in flat relational algebra (Paredaens, Van Gucht 1992; Wong 1996). [U]
- Query shredding compiles nested-result queries to a fixed number of flat queries. Cheney, Lindley, Wadler, SIGMOD 2014. [U]

## Query language

- Wong, "Kleisli, a functional query system", JFP 2000. [S 2026-10-25] https://dl.acm.org/doi/10.1017/S0956796899003585
- Buneman, Libkin, Suciu, Tannen, Wong, "Comprehension syntax", SIGMOD Record 1994. [S 2026-10-25] https://dl.acm.org/doi/abs/10.1145/181550.181564
- CPL (Kleisli surface) writes sets `{}`, bags `{| |}`, lists `[ ]` with one comprehension form. [U]
