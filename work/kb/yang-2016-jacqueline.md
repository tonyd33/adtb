# Jacqueline (Yang et al., PLDI 2016)

kind: paper
home: https://people.seas.harvard.edu/~chong/pubs/pldi16-jacqueline.pdf

## Model

- Policy-agnostic Django-like framework: policies attached to fields with `@label_for`, taking the row and the viewer; the rest of the program has no checks. Faceted execution simulates executions for each label assignment; proof of termination-insensitive non-interference and policy compliance for core calculus λJDB. [V 2026-10-25] pdftotext of paper, sections 1-2
- Faceted ORM stores each facet as its own row with meta-columns jid (shared id) and jvars (label assignment, e.g. "k=True"); foreign keys use jid; plain relational selects, joins and sorts then return correctly guarded facets. [V 2026-10-25] section 3.1
- Early Pruning: once the viewer is known (session user), unneeded facets are dropped, reducing overhead. [V 2026-10-25] section 3.2
