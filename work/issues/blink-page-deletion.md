---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 541
summary: "B-link pages are never freed; deleting them needs a rule, since descents rely on page ids staying valid"
closed: 2026-10-25
depends-on:
  - blink-space-reuse
tags:
  - engine
  - design
---

# Feature: B-link page deletion

## Report

Optimistic descents hold nothing across levels, and they rely on a page
id they read keeping its tree and low key. Freeing an empty page breaks
that. A rule is needed first, for example half-dead pages unlinked and
freed only after every descent that could hold their id has finished,
using the watermark of epochs. State it in `docs/design/indexes.md`
§8.1 before implementing.

## 2026-10-25: resolved

Landed as `feat(btree): unlink emptied leaves by fence keys, keep postings and clean up before splitting`. Pages carry fence keys; an emptied leaf is unlinked and freed in one batch. Leaf merges stay ruled out by `btree/merge/empty-only`. Follow-ups are in `blink-follow-ups.md`.
