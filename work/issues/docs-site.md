---
created-at: 2026-10-26
status: in-progress
kind: feature
priority: 881
summary: "the manual, spec and design docs are markdown files in the repo with no site to read them in a browser"
branch: docs-site-research
depends-on:
  - user-manual
tags:
  - docs
  - tooling
---

# Feature: A docs site

## Report

Tony: I should be able to view the docs in a browser.

Build a static site from `docs/manual/`, `docs/spec/` and `docs/design/`, with the manual first and the spec and design files as their own sections. Astro with Starlight is the first candidate (markdown in place, search, sidebar from the file tree, dark mode); pandoc or mdBook are the fallbacks if Starlight cannot take the files as they are. Compare them on the real files before choosing, and record the comparison in `work/kb/`.

Requirements:
- The markdown stays where it is and stays the source. The site reads it; it does not need copies or front matter edits beyond what the generator strictly needs, and none of that may break reading the files on Tangled or in an editor.
- Relative links between files and to section anchors (`syntax.md#83-...`) work on the site, and a link check in the suite fails on a broken one.
- `adt` code blocks are highlighted. Shiki-based generators take a TextMate grammar, not tree-sitter, so either write a small TextMate grammar for adtb or generate one; keep it in step with `tree-sitter-grammar` by testing both against the same files.
- Full-text search across all three sections.
- The site builds with one command in the devenv shell and serves locally as a devenv process, like quandary's.
- A build of the site can be published: pick a host (Tangled, GitHub Pages or a static host Tony names) and document the one command that publishes it. Publishing stays a manual step Tony runs.

Done when the site builds and serves locally with every page of the three sections, the link check passes in the default test run, and Tony has read the manual's getting-started page on it in a browser.
