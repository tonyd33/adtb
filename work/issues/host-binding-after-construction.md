---
created-at: 2026-10-27
status: open
kind: bug
priority: 895
summary: "host function bodies are bound by mutating compiled programs, scalars and index keys after construction at eight sites, so a site that forgets gets MissingFunction at run time"
tags:
  - quality
  - engine
---

# Host function bodies are bound by mutating compiled values after construction

## Report

`Scalar::new`, `IndexKeys::new` and `exec::compile` take no host functions; each caller binds them afterwards. The sites: `crates/adtb/src/catalog/keying.rs:253`, `catalog/moved.rs:140`, `catalog/validate.rs:149`, `catalog/hosts.rs:10-17`, `write/mod.rs:444` and `:464`, `write/eval.rs:355`, `api/mod.rs:481`. `Catalog::open` builds index keys with empty hosts (`catalog/mod.rs:576`, `:694`, `:712`), which work only because `api::open` calls `set_hosts` before anything evaluates. A new construction site that forgets to bind gets `MissingFunction` at run time, not a compile error.

Fix: pass the hosts to the constructors, so an unbound value cannot be built.
