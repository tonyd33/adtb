---
created-at: 2026-10-30
status: open
kind: feature
priority: 862
summary: "no language server: editors get no diagnostics, go to definition, hover or completion for .adt and .adb files"
---

# Feature: Language server

## Request

## Proposal

## Open questions

An LSP server for `.adt` and `.adb` files over `adtb-lang`, with no engine or running database needed. Diagnostics at the front end's spans, go to definition, hover with types, completion, document symbols.

Report the front end's errors as they are, so the editor and `adtb-lang` never disagree. Editing needs a front end that recovers from errors and answers for a partial file, which it may not do today: find out first. The tree-sitter grammar (tree-sitter-grammar) serves highlighting and folding; this does not repeat them. A new dependency for the protocol needs a justification in `deny.toml`.

Done when an editor shows the same diagnostics as the parser and checker on every file under `conformance/` and `apps/campaign/`, each request has tests driven over the protocol, and it builds in the dev shell.
