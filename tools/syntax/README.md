# Shared syntax corpus

`corpus/*.adt` and their paired JSON span expectations are the common inputs
for syntax highlighters. Each expectation is a Highlight.js token class and
exact source text.

`work/tools/tree-sitter-check` runs the tree-sitter grammar's highlight query over
the same `.adt` inputs and maps its captures to these expectations. Highlighting
accepts fragments and elisions; these files do not claim to be complete programs.
