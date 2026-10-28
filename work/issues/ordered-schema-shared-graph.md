---
created-at: 2026-10-28
status: resolved
kind: bug
priority: 699
summary: ordered type checking repeatedly expands shared type graphs
closed: 2026-10-28
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

## 2026-10-28: exact baseline and reviewed language repair

Baseline92666609f36238fc7da94847098baa16497ccd80 on55220629 reproduced the valid
finite nested Option refusal (exit100,0.008s) and the30-level shared type graph
timeout (exit100,60.005s). Temporary stage markers reached parse/build/
check_ordered entered but never completion, proving the schema stage independently
of principal validation. Total110.862419s including50.30s cold compilation,
16:12:07Z–16:13:58Z. All tracked/local config hashes and revision unchanged.
Baseline logs/receipt retained under target/ordered-baseline.

Final source e4460037e04980f8ce7a1b1713d5e959102bcd76/base55220629 changes only
schema.rs and schema_tests.rs. Query-local declaration requirements are composed
through actual args and the current parameter callback; declaration and completed
key requirements are memoized separately. Active guards reject genuine cycles
without expanding growing args. No persistent Schema cache or public signature,
dependency or normative change. Newtype base-parameter admission remains rejected;
missing substitution there was not an admitted bug. No fixture/timeout bypass.

All assigned language gates passed0 on stable final inputs:659tests including
641unit+9spec+9textual, strict lang clippy, fmt and house; additional size0.
Tests2.124s, total37.546583s,16:21:45Z–16:22:22Z. Seven new regressions each
0.009–0.014s; nominal30-level graph0.012s, composite-key graph0.011s. Generic,
phantom/required-param, callback/schema-state and recursive/key controls passed.
No separate old-key failing executable baseline is claimed. Key tests concern
language ordering, not materialization of huge engine keys. Rust1.98.1/Cargo1.98.0,
configured defaults. Log SHA256f5ef656bd17a5cd44983debc74fa6cfe728de35deab038505353fb350efa071d.

Fresh independent exact-source review reports no blocker; report/receipts under
work/briefs/review-ordered-schema.md and receipt-ordered-schema.md. Author's
branch full engine/campaign/docs check was explicitly assigned to lead instead;
post-land main integration is still REQUIRED, not yet passed. Principal validator
remains frozen and must rebase/rerun its unchanged legitimate DAG fixture only
after this repair and main integration.

## 2026-10-28: resolved

Landed928b7d38, fix(lang): bound ordered schema graph traversal, after exact
independent review and all659language checks. Lead broader main check passed0
on clean797531477236bb9b894e9e5999781b556e115640, parent928b7d38. Exact root
command: direnv exec /home/tony/code/adtb env DEVENV_ROOT="$PWD"
CARGO_TARGET_DIR="$PWD/target" check.16:27:02Z–16:33:50Z,408s; all tracked/local
root+app Cargo config hashes and revision stable. Rust1.98.1/Cargo1.98.0,
configured defaults; permission/queue/execution-only unknown.

Engine2,373tests/8skipped49.912s; campaign47/1skipped21.140s; all lint, dependency,
size and native docs checks passed. Docs10tests/420syntax/61pages/6730links/
4995fragments/0failures. Main log target/ordered-schema-integration/check.log
SHA2567d8c0817e21e6b744844318130f9947dbecf5b605ceeb5e1417f4862e2b0019f.
Post-rebase delta versus reviewedbranch is only four ticket/index metadata files.
Receipts/review: work/briefs/{receipt-ordered-schema,receipt-ordered-schema-main,
review-ordered-schema}.md; worker evidence backed up and SHA256verified under
target/ordered-schema-evidence, original source/logs retained.

Principal validator build slot released after mainpass. Its original legitimate
30-level fixture remains unchanged and must pass affected/full verification on
the rebased source. This closes the general ordering issue, not the separate
principal or provider tickets.

## 2026-10-28: resolved
