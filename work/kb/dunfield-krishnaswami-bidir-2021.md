# Dunfield and Krishnaswami, Bidirectional Typing (survey)

kind: paper
home: https://arxiv.org/abs/1908.05839

## Bidirectional typing

- Combines checking (type is input) and synthesis (type is output); checking supports features whose inference is undecidable, synthesis avoids the annotation burden; improves error locality. [V 2026-10-25] arXiv 1908.05839v2 abstract (ACM CSUR 2021)
- Pfenning recipe: introduction rules check, elimination rules synthesize (principal judgment of an elimination is its first premise); subsumption is the change of direction, replacing equality with subtyping. [V 2026-10-25] sec. 4
- Criterion "annotation character": whether a needed annotation can be predicted by looking at the subterm, not the whole program. [V 2026-10-25] sec. 3
