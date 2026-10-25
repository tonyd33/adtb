---
title: How the engine knows every reader has a new constructor
pick: some
raised-by: [evolution/old-readers/readers-first]
options:
  handshake:
    title: Each connection declares the schema version its client was built for
  bundles:
    title: Each client release registers a query bundle tagged with its version
  manual:
    title: An operator enables the constructor for writes
links:
  - {rel: prior-art, to: prior/surface-schema-registry}
---
`Apple` is declared, then enabled for writes once no reader would meet
it untyped.

```
-- 0009
add constructor Identity.Apple { sub: Text }

-- 0010
enable identities.Apple
```

## surface/reader-tracking/handshake

The client sends its version on connect; the engine keeps the minimum
over live connections. Disconnected clients are invisible.

## surface/reader-tracking/bundles

A bundle names its version; `enable` refuses while an active bundle
predates `Apple`. Confluent BACKWARD: consumers before producers.

## surface/reader-tracking/manual

The engine trusts the operator.
