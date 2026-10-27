---
created-at: 2026-10-27
status: open
kind: feature
priority: 842
summary: "embedding.md does not say what a non-set query returns, and its typed API differs from what the generated code needs"
tags:
  - spec
---

# Feature: embedding spec gaps found by code generation

## Report

- `embedding.md` says a call returns `Rows` of its element type, which is undefined for a query whose result is not a set. Ruling: a query of a non-set type returns its value, as a function call does; state it in section 4.1.
- `Bundle<B>` and `Transaction` in the spec carry neither the runtime parameter nor the transaction's lifetime the Rust binding needs. Update the signatures to `Bundle<B, R>` and `Transaction<'a, B, R>`.
- `Transaction::query_text` takes `(&str, Ty, Value)` parameters in the engine, the spec `(&str, Value)`. Ruling: the spec's form; the engine infers each parameter's type from the text and refuses a value that does not fit.
