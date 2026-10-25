---
title: Some host languages have no closed sums
pick: any
raised-by: [surface/codegen/schema-types, surface/codegen/query-functions]
options:
  sealed:
    title: Sealed interfaces or a visitor per sum
    links:
      - {rel: prior-art, to: prior/surface-protobuf-oneof}
  tagged-struct:
    title: A tag enum plus one optional field per constructor
  live:
    title: Live with generating only for languages that have sums
    excludes: [surface/codegen/no-sums/sealed, surface/codegen/no-sums/tagged-struct]
---
Go and C lack them; Java has them only since sealed classes
(https://openjdk.org/jeps/409).

## surface/codegen/no-sums/sealed

```go
type Identity interface{ isIdentity() }
type EmailPass struct{ Email string }
type Google struct{ Gid string }
```

protobuf-go oneof wrappers. Exhaustiveness rests on a linter.

## surface/codegen/no-sums/tagged-struct

```c
struct Identity { enum { EMAIL_PASS, GOOGLE } tag; struct EmailPass *email_pass; struct Google *google; };
```

Reintroduces the nullable-fields encoding the engine refused, in the
client.

## surface/codegen/no-sums/live

Rust, TypeScript, Kotlin, Swift, Haskell, OCaml only.
