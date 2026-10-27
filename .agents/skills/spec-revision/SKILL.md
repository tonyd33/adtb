---
name: spec-revision
description: Revise the adtb specification in docs/spec/. Give each spec file one writer, record design-level decisions in the design space, run the executable example checks before any reading review, and review with fresh agents through an implementability lens. Use when writing, revising or reviewing spec parts, or when implementation work reports a spec gap.
---

# Spec revision

## Write

- When several agents write, assign each spec file to exactly one of
  them. Changes to a file go through that agent.
- Record each design-level decision as a question under
  `work/research/questions/spec/` in the design space, not only in the
  prose.
- Name the tier of every change: what users observe goes in `docs/spec/`,
  what another subsystem relies on goes in `docs/design/`, everything
  else stays in code. Reject a change whose only purpose is to make an
  implementation easier.

## Check before review

Run the executable checks first. Reading reviews miss what these find:
productions that clash when composed, grammar that contradicts prose, and
examples that do not parse or type-check.

```
cargo nextest run -p adtb-lang --test spec_corpus
work/tools/conf conformance
```

## Review

- Review each changed area with a fresh agent that did not draft it.
- Ask reviewers to implement, not only to read: "Write the algorithm this
  rule describes. List the inputs it does not decide."
- A falling count of reviewer findings does not show the spec is
  complete. Expect implementation to find more.
- Triage every finding: a ruling goes through the design space, a fix
  goes to the file's owner. Finish with one cross-file pass over links
  and terms.

## Gaps found by implementation

A gap found while implementing is a spec bug. Route it through the design
space with a concrete program or history that the spec leaves undecided.
