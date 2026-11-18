---
created-at: 2026-11-18
status: open
kind: feature
priority: 28
tags: [tooling]
summary: work/ becomes its own local repo; the public repo references nothing under work/
---

# Feature: Work repo

## Request

Maintainer rulings: work/ becomes its own repo, gitignored in the public repo, kept local and never pushed; the public repo references nothing under work/ (work/briefs/decisions-tooling-crates-2026-11-17.md). 2026-11-18: the maintainer workflow (the AGENTS.md sections on tickets, handoff, knowledge base and design space; the lead, quality-review, design-space and spec-revision skills) moves into work/AGENTS.md and work/skills/, loaded by a gitignored CLAUDE.local.md that imports @work/AGENTS.md. The work repo keeps work/'s history (git subtree split before the public commit untracks it).
