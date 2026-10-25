---
title: Whether the test suite is part of the spec
pick: one
options:
  normative-suite:
    title: The official suite is normative; passing it is part of conforming
    links:
      - {rel: prior-art, to: prior/project-wasm-spec}
  informative-suite:
    title: A published suite that illustrates the spec but does not bind
  none:
    title: Tests belong to the implementation only
---
Prose and a test disagree sooner or later; the spec must say which
wins.

## project/spec/conformance/normative-suite

WebAssembly's test suite lives with the spec and every engine runs it.
A disagreement between prose and suite is a spec bug fixed in both.

## project/spec/conformance/informative-suite

Examples published with the spec, maintained by the implementation;
prose wins.

## project/spec/conformance/none

SQLite's TH3 is proprietary; the implementation's tests are its own.
