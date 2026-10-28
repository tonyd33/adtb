# Pandoc

kind: system
home: https://pandoc.org/

## Documentation sites

- Pandoc converts Markdown to standalone HTML with -s; filters can transform the parsed AST. It provides table of contents, templates, and custom language highlighting through KDE XML syntax definitions. Search/navigation for a multipage docs site need additional machinery. [V 2026-10-27] https://pandoc.org/MANUAL.html#options https://pandoc.org/MANUAL.html#syntax-highlighting
- adtb comparison rendered all 57 unchanged Markdown pages with pandoc 3.7.0.2 (`-f gfm -t html5 -s`) in 21.87 s. Baseline leaves 1,414 internal .md hrefs unresolved in HTML output. This can be fixed with a filter, but integrated search and multipage navigation would still be separate work; no claim that these are impossible. [V 2026-10-27] Local evidence: target/docs-prototype/pandoc-out-links.txt in docs-site-research workspace; https://pandoc.org/MANUAL.html#options
