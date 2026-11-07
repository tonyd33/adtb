---
created-at: 2026-11-06
status: open
kind: feature
priority: 100
summary: "a targeted adtb-sim run after an edit rebuilds the test binary in a median 114-124 s, 80% of nextest time and 38% of author time on today's tracks; build jobs past 3 do not shorten it"
---

# Feature: Cut the adtb-sim rebuild after an edit

## Request

## Proposal

## Open questions

## 2026-11-06

From the timing study of 2026-11-06 (work/briefs/review-devex3.md, local): across 89 nextest logs 80% of the time is compile; a rebuild of the adtb-sim test binary after an edit has a median of 114-124 s at 6 jobs alone, the same 114 s at 3 jobs, 174 s at 3 jobs beside another track. It was 50 to 123 min per author round on the four engine tracks. Measure where a rebuild goes (the adtb crate, adtb-sim's test crate, the link) and try the cheap levers first: a faster linker (mold or lld), codegen units and incremental settings for the test profile, splitting the slowest modules out of the adtb crate's rebuild path. Report the rebuild time per lever.

## 2026-11-06: prioritized

Maintainer, 2026-11-06: prioritize this. Next track to dispatch.
