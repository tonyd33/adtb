---
created-at: 2026-10-25
status: open
kind: feature
priority: 97
summary: "the conformance suite runs only on the reference interpreter"
tags: [quality]
---

# Feature: run the conformance suite on the engine

## Report

adtb-conformance runs its 265 files against the reference only. Add a second runner over the engine with a checked-in list of files the reference passes and the engine fails. The list may only shrink: a file leaving it is a pass, a file joining it fails the run. This is the first part of engine-differential-tests.
