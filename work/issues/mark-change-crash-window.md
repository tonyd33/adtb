---
created-at: 2026-11-06
status: open
kind: bug
priority: 560
summary: "change_marks changes the in-memory marks before logging the mark, so a crash between loses the mark and a record whose evidence vanished"
tags:
  - class-damage-unreported
---

# A crash inside change_marks loses the mark and a vanished record

## Report

## 2026-11-06

From the damage-log delta review (work/briefs/review-damage-log-r1-delta-report.md, D3). `change_marks` changes the marks set before the mark is logged; a crash in between loses the mark, and with it a damage record whose evidence also vanished. The mark had not been acknowledged. Direction: log the mark before the in-memory change, or derive marks from records (damage-marked-at-detection change 2) so the window carries nothing.
