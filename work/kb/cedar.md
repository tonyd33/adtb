# Cedar (AWS policy language)

kind: language
home: https://github.com/cedar-policy

## Verification-guided development

- Executable Lean models of evaluator, authorizer and validator serve as the specification; proofs on the models; Rust production code checked against the models by differential random testing (DRT); property-based testing for unmodeled parts. [V 2026-10-25] Disselkoen et al., "How We Built Cedar: A Verification-Guided Approach", arXiv:2407.01688
- Models favour readability and are kept feature complete so proofs cover the full language and the models work as test oracles. Proof-to-model ratio about 3.4:1. [V 2026-10-25] same
- Proofs found 4 validator bugs; DRT and PBT found 21 more. [V 2026-10-25] same
- Rejected writing production code in Lean and shipping generated C. [V 2026-10-25] same
