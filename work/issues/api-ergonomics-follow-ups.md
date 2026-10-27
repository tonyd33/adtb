---
created-at: 2026-10-27
status: open
kind: feature
priority: 844
summary: "API errors do not implement std::error::Error, generated code warns dead_code, codegen parse errors run together, and nothing computes a bundle's pin hash"
tags:
  - engine
  - lang
---

# Feature: API ergonomics found by the manual

## Report

- `adtb::Error` and `OpenError` do not implement `std::error::Error`, and a generated bundle `Error` has no `Display`, so `?` into `Box<dyn Error>` does not work.
- Generated code raises `dead_code` warnings in a program that uses only part of it.
- Codegen parse errors run together with no separator: "found `by`expected `fn`...".
- `HostFnMismatch` prints "is bound at another type" when only the version differs.
- A migration refused for a missing `add module` prints the same reason three times.
- Nothing computes a bundle's pin hash for its author; the manual has readers fetch it from the shell. Add it to `adtb-codegen` or the shell.
- `by` is a keyword, so it cannot be a parameter name, though it is a field name.
