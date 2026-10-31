---
created-at: 2026-10-30
status: resolved
kind: feature
priority: 884
summary: "tools/docs-site wraps mdbook with a hand-written link checker, a highlighter test and a test of mdbook serve; keep only what mdbook cannot do"
closed: 2026-10-30
---

# Feature: Docs site trim

## Request

## Proposal

## Open questions

Maintainer: this reads as hacky on top of mdbook. The lead's verdict, by reading:

- `links.py`: replace with an existing checker of built HTML that checks fragments and runs offline, such as lychee, if it is in nixpkgs and gives the same failures. mdbook checks no links, and a broken-link check stays a requirement of docs-site.
- `test_serve.py`, 160 lines: tests mdbook's own serve. Remove what does not test this repository.
- `syntax.py` and `syntax-check.js`: keep the keyword comparison with the lexer. Drop the token span corpus if tree-sitter-grammar's checks cover those inputs. The site's highlight.js grammar stays, since mdbook uses highlight.js.
- `navigation.py`: remove, with its test and preprocessor entry. List every page in `docs/SUMMARY.md`, as mdBook expects. It parses the section READMEs to fill in a SUMMARY.md that already exists. mdBook does not fail on an unlisted page, so state how a page left out gets noticed, at most a few lines.
- `check.py`: becomes the few lines left, or a step in `check`.

No Python stays. The keyword comparison and the check for a page left out of SUMMARY.md become Rust tests in the suite, with the keyword list read from the lexer's own table rather than by a regex over lex.rs. `work/tools/docs-site` keeps build and serve.

Done when no Python is left under `tools/docs-site/`, and `tools/docs-site/` holds only what mdbook and a stock checker cannot do, `book.toml` adds no custom preprocessor, and a broken link and a keyword missing from the highlighter each still fail `check`.

## 2026-10-30: resolved

`tools/docs-site/` holds only `adt-highlight.js`; lychee checks links and fragments offline. `use-default-preprocessors` stays false, since the index preprocessor renames README pages while links keep `README.html`, which breaks 49 links (work/kb/mdbook.md). Accepted, with the tree-sitter checks not landed: nothing runs the highlight.js grammar, so a syntax error in `adt-highlight.js` outside its keyword string passes `check`; lychee does not check `<script src>`, which mdBook generates. Page order in SUMMARY.md and the section READMEs can drift. Branch check exit 0, 3420 tests.

## 2026-10-30: resolved
