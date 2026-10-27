---
created-at: 2026-10-27
status: open
kind: feature
priority: 841
summary: "generated code does not compile for an anonymous type inside a generic declared type, maps a restricted constructor set to the full enum, and refuses user types named like Rust prelude types"
tags:
  - lang
---

# Feature: code generation follow-ups

## Report

- An anonymous record or sum inside a generic declared type does not carry the type's parameters, so the emitted code does not compile. Generic and recursive declared types have no fixture in `crates/adtb-codegen-tests`.
- A restricted constructor-set type such as `identities.Google` maps to the full enum.
- A user type named `Error`, `Option`, `Result` or another Rust prelude name is refused instead of being emitted under a qualified path.
- `call_arrow` returns the untyped `adtb::ArrowStream`; a typed Arrow result needs the `arrow` crate, which is not approved.

## 2026-10-27: keys across bundle versions

Each bundle version gets its own relation marker types, so a key carried from a `v1_0` call to a `v1_3` call needs `Key(k.0)`. Share relation markers between bundle versions of one log where the relation is unchanged.
