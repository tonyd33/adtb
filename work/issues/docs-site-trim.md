---
created-at: 2026-10-30
status: open
kind: feature
priority: 884
summary: "tools/docs-site wraps mdbook with a hand-written link checker, a highlighter test and a test of mdbook serve; keep only what mdbook cannot do"
---

# Feature: Docs site trim

## Request

## Proposal

## Open questions

Maintainer: this reads as hacky on top of mdbook. The lead's verdict, by reading:

- `links.py`: replace with an existing checker of built HTML that checks fragments and runs offline, such as lychee, if it is in nixpkgs and gives the same failures. mdbook checks no links, and a broken-link check stays a requirement of docs-site.
- `test_serve.py`, 160 lines: tests mdbook's own serve. Remove what does not test this repository.
- `syntax.py` and `syntax-check.js`: keep the keyword comparison with the lexer. Drop the token span corpus if tree-sitter-grammar's checks cover those inputs. The site's highlight.js grammar stays, since mdbook uses highlight.js.
- `navigation.py`: keep. It builds the book's navigation so that the Markdown needs no SUMMARY.
- `check.py`: becomes the few lines left, or a step in `check`.

Done when `tools/docs-site/` holds only what mdbook and a stock checker cannot do, and a broken link and a keyword missing from the highlighter each still fail `check`.
