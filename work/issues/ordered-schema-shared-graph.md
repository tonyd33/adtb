---
created-at: 2026-10-28
status: in-progress
kind: bug
priority: 699
summary: ordered type checking repeatedly expands shared type graphs
branch: ordered-schema
tags:
  - engine
  - lang
  - quality
  - test
---

# Ordered schema shared graph

## Report

A legitimate shared ordered type graph Tn={a:T(n-1),b:T(n-1)} caused the
principal validator unit fixture to time out60.004s before its validation call
oncaab9c70/basec14a6594. Other three generic value tests passed. Exact main-stage
isolation is still required; source review locates Schema builder check_ordered
at schema.rs1167 and ordered_at506, whose path-only seen stack is popped after
each child, producing repeated graph unfolding. check_ordered2142 invokes it for
each field/type. Logs/unchanged IDs in principal-qualification-corrected, total
87.219s including compilation. This is not a passed validator/deep-DAG test.

Fresh ordered-schema workspace on55220629 owns a minimal language-stage witness,
general instantiated/context-correct traversal repair and focused regressions.
Do not bypass ordered checks with trusted flags or reduce the legitimate caller
fixture to hide the timeout. Investigate generic newtype substitution and finite
nested instantiation alongside memo key/cycle safety, proving behavior first.
No new public signature, dependencies or normative choices.

Landing order: ordered-schema first, lead full main integration, then principal
validator rebases and reruns its unchanged legitimate fixture/focused/full checks;
role provider remains paused. Author owns all language tests/spec_corpus, strict
lang clippy/fmt/house and measured bounded regression, fresh exact review. Lead
explicitly owns broader root/campaign/docs check after landing, avoiding another
full cold engine build in this language workspace. Evidence must retain exact
source/base/hash stability, commands/final exits/logs/timings and unfinished scopes.
