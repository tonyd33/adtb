---
created-at: 2026-11-18
status: open
kind: feature
priority: 26
tags: [test]
summary: assert the 144 weak refusal sites with refused_as so the TQL refusal rule is zero-match
---

# Feature: Refusal sites

## Request

Maintainer ruling, 2026-11-18 (work/briefs/tql-session-2026-11-18.md): refusal-check becomes the TQL rule work/briefs/tql/refusal.tql, zero-match. It finds 144 sites today: asserts of is_err, is_ok, matches!(.., Err(_)) or Ok(_), contains/starts_with/ends_with on error text, bare unwrap_err()/expect_err(..) statements, calls of refused_with. Each becomes an exact assertion of the refusal value and whole reason with adtb_testkit::refused_as.
