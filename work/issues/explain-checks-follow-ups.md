---
created-at: 2026-10-29
status: open
kind: feature
priority: 640
summary: "adtb-probe --checks takes only qualified names, labels nested-read checks Changed, and does not name the change-set input a check starts from"
---

# Feature: constraint check explain follow-ups

depends on: none
needed by: none

## Request

Gaps the explain-constraint-checks writer reported:

1. `adtb-probe --checks` and `Database::explain_checks` take only the qualified name (`app::adults`); a bare `adults` is reported unknown even when it names one constraint.
2. A check whose read is nested runs on `Part::Changed`, inserts and deletes together, so its label does not name one side.
3. `plan::explain::Describe` prints `One row` or `Unnest` with no detail for the change-set input a check reads, so the plan does not show which input it starts from.

## Proposal

Resolve a bare name when it is unique among the schema's constraints and checks. Name the change-set input (relation and part) in the node `Describe` prints for it. Keep `Changed` labels and print them as "any change to".

## Open questions

None.
