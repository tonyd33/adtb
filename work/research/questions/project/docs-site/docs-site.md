---
title: Generator for the local documentation site
pick: one
options:
  mdbook:
    title: mdBook with generated navigation and an adt Highlight.js grammar
  starlight:
    title: Astro Starlight with a source loader and Markdown link adapter
  pandoc:
    title: Pandoc with custom navigation and search integration
---
Render docs/manual, docs/spec and docs/design locally without moving or
rewriting their Markdown sources. Preserve relative file links and anchors,
provide full-text search and adt highlighting, and integrate build and serve
with devenv. Hosting is a separate task (work/issues/docs-site-hosting.md).

The 2026-10-27 isolated research track tested the real source files. Commands,
versions and primary sources are recorded in work/kb and
work/briefs/handoff-docs-site-research.md. Implementation must repeat these
checks rather than treat prototype success as production acceptance.

## project/docs-site/mdbook

Use pinned mdBook from the development environment. Generate SUMMARY.md and
symlink the source sections into ignored staging output. Disable default
preprocessors and explicitly enable links so README.md remains README.html.
Generate a small root landing page. Register a small adt Highlight.js grammar
against a shared syntax corpus, ready for the future tree-sitter grammar.

The adapted prototype reports zero failures across 2,985 emitted internal
links, including 2,336 fragments. Chromium verified search results from all
three sections, a numeric section anchor and highlighted adt spans. Build
cost is small; no Node dependency tree or title metadata adapter is required.
The integration must pin compatible mdBook/Highlight.js interfaces and test
links and grammar behavior. The staged source links are an internal build
artifact, never an additional authored copy of the documentation.

## project/docs-site/starlight

Use pinned Astro and Starlight. Wrap its glob loader to supply titles from
first headings, rewrite source .md links to output routes and supply an adt
TextMate grammar. The prototype passes its emitted link/fragment checks and
search. It adds Node and an npm dependency tree of roughly 354 packages.
Existing first headings also need presentation care to avoid duplicate titles
and search entries. This is viable, but its extra machinery does not provide
a required local-site capability absent from the adapted mdBook prototype.

## project/docs-site/pandoc

Render pages with Pandoc, adding navigation, route/link rewriting, search and
an adt syntax definition. The raw prototype renders the pages but retains
source .md hrefs and has no integrated search/navigation. These require
additional application code. It is viable with that work, but provides less
of the required local docs experience than either documentation generator.
