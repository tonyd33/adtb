# Shared syntax corpus

`corpus/*.adt` and their paired JSON span expectations are the common inputs
for syntax highlighters. Each expectation is a Highlight.js token class and
exact source text. The docs-site check executes its grammar against these
fixtures, every lexer keyword and all `adt` fences in the three docs sections.
It checks token spans, text preservation and keyword coverage against the lexer.

A future tree-sitter grammar must consume the same `.adt` inputs and map its
captures to these expectations. No tree-sitter backend exists yet. Highlighting
accepts fragments and elisions; these files do not claim to be complete programs.
