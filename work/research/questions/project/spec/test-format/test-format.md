---
title: How a conformance test is written
pick: one
raised-by: [project/spec/conformance/normative-suite, project/spec/conformance/informative-suite]
options:
  script:
    title: A script of statements and queries with expected rows, as sqllogictest
  in-language:
    title: Tests as queries in the language that must return nothing
  golden:
    title: Input files with recorded output files
---
The suite outlives any one implementation and must be readable as
part of the spec.

## project/spec/test-format/script

SQLite's SQL Logic Test and its Rust runner (sqllogictest-rs), used to
compare engines; WebAssembly's `.wast` scripts of modules and
assertions.

```
schema
  type Identity = EmailPass { email: Text } | Google { sub: Text }
  relation identities : Identity key id
run
  insert identities.Google { id: 1, sub: "g1" }
query rowsort
  from i in identities select match i | EmailPass e -> e.email | Google g -> g.sub
----
g1
error MatchNotExhaustive
  from i in identities select match i | Google g -> g.sub
```

## project/spec/test-format/in-language

Expected results are relations written in the language; a test is a
constraint-shaped query (`constraint/form/empty`) that returns the
difference.

```
test google_only:
  (from i in identities select match i | EmailPass e -> e.email | Google g -> g.sub)
  symdiff { "g1" }
```

Uses the engine to judge itself; a bug in set difference hides others.

## project/spec/test-format/golden

Recorded outputs; regenerated on intended change. Says what happened,
not what should.
