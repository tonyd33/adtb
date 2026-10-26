---
created-at: 2026-10-25
status: in-progress
kind: feature
priority: 166
summary: "the canonical name m::r.unique#n uses a catalog id the front-end schema does not carry, so the interpreter uses the unique's position"
branch: adtb-cat
tags:
  - lang
---

# Feature: catalog ids in the front-end schema

## Report

The canonical name of a unique, `m::r.unique#n`, uses its catalog id.
The front-end schema carries no catalog ids, so the interpreter uses
the unique's position among the relation's uniques as a stand-in. The
names diverge once a unique is dropped or added out of order.
