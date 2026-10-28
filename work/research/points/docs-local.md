---
extends: current
proposed: true
accept:
  project/docs-site/mdbook: {title: mdBook with generated navigation and an adt Highlight.js grammar, reason: 'The real-file prototype preserves links and anchors, supplies search and adt highlighting, and fits the Rust/Nix environment with less custom integration and dependency maintenance than Starlight or Pandoc. Hosting remains separate.'}
---
