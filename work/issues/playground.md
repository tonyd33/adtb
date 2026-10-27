---
created-at: 2026-10-26
status: open
kind: feature
priority: 882
summary: "there is no way to try adtb without building it: no playground that runs schemas and queries in a browser"
depends-on:
  - docs-site
tags:
  - docs
  - tooling
---

# Feature: An online playground

## Report

A page on the docs site where a visitor writes a schema, inserts rows and runs queries, migrations and transactions, and sees results, with nothing installed.

Run the engine itself in the browser, compiled to WebAssembly, not a server: the deterministic simulator's in-memory I/O and single-threaded runtime are already the shape a browser needs. A database lives in memory for the page's lifetime, with an optional save to browser storage. Ad hoc text goes through the same entry point the `adtb` shell uses, so the playground and the shell agree.

Requirements:
- `adtb`, `adtb-lang` and what they need build for `wasm32-unknown-unknown`, with a runtime and `Io` behind the existing traits that need no threads, clock or file system from the host. A CI-style test in the suite builds the wasm target so it cannot rot.
- An editor with `adt` highlighting and the front end's errors shown at their spans. CodeMirror 6 or Monaco; the tree-sitter grammar's wasm build is an option for highlighting.
- Examples from the manual open in the playground with one click, and the page can share a session as a link that encodes the schema and statements.
- Results show nested rows and constructors as the shell prints them, and refusals as the shell's messages.
- The bundle stays small enough to load on a phone: report its size, compressed, and keep a budget for it in the suite.

Done when the manual's getting-started session runs unchanged in the playground, a shared link reproduces a session, and the wasm build is part of the default check.
