---
title: Builder-composed queries are invisible until they run
pick: any
raised-by: [[surface/codegen/builder, evolution/breaks/invisible/registered]]
options:
  static:
    title: The builder admits only shapes extracted at build time
  adhoc-class:
    title: Builder queries are ad hoc, checked at execution only
  live:
    title: Live with builder queries escaping the recheck
    excludes: [surface/codegen/dynamic/static, surface/codegen/dynamic/adhoc-class]
---
A builder composes queries from runtime values, so the set of queries
a client sends is not known when it registers.

## surface/codegen/dynamic/static

A compiler plugin or macro evaluates each builder expression at build
time and registers the result; dynamic composition is a compile error.
Rust macros make this natural for the embedded API.

## surface/codegen/dynamic/adhoc-class

The builder emits ad hoc queries under whatever ad hoc rule holds.

## surface/codegen/dynamic/live

The registered guarantee covers only queries not built by the builder.
