# Safe Serializable Secure Scheduling (Sheff, Magrino, Liu, Myers, van Renesse, CSF 2016)

kind: paper
home: https://arxiv.org/abs/1608.04841

## Abort channels

- Coordination for serializable transactions leaks: abort channels carry at most one bit per attempted transaction but can be a covert channel; Spanner and Azure Storage "in principle vulnerable"; timing mitigation does not control them. Proposes staged commit with a static check. [V 2026-10-25] pdftotext of arXiv v2, sections 1-2
