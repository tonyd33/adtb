---
title: How values cross into a WebAssembly function
pick: one
raised-by: [interop/udf/wasm]
options:
  wit:
    title: Component-model WIT types; sums are WIT variants
    links:
      - {rel: prior-art, to: prior/interop-wasm-udf}
  bytes:
    title: Arguments and results as bytes in the client wire encoding
  arrow:
    title: A batch of Arrow arrays in linear memory per call
    links:
      - {rel: gains, to: exec/batch/arrow}
---
A function taking `Identity` needs the constructor and payload; the
call cost decides whether one call per row is tolerable.

## interop/udf/abi/wit

```
variant identity { email-pass(email-pass-row), google(google-row) }
score: func(i: identity) -> f64;
```

WIT `variant` has named cases with optional payloads, plus `enum`,
`option`, `result`, `record`
(https://component-model.bytecodealliance.org/design/wit.html). The
engine generates the WIT from the schema; nominal sums map one to one.

## interop/udf/abi/bytes

Extism passes bytes in and out
(https://github.com/extism/extism). The guest decodes with the
codegen of `surface/codegen/schema-types`.

## interop/udf/abi/arrow

One call per batch amortises the boundary; sums arrive as dense
unions. The guest author writes a vectorized kernel.
