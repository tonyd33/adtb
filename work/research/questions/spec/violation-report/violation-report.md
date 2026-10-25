---
title: "What a constraint failure reports"
pick: one
options:
  one:
    title: "The violated constraint with the smallest name, its report as descriptor and rows, unreadable rows redacted with a count"
  all:
    title: "Every violated constraint"
---
Raised by the second review of the spec in docs/spec/.

## spec/violation-report/one

The violated constraint with the smallest name, its report as descriptor and rows, unreadable rows redacted with a count. An error value is not a collection; one deterministic constraint suffices to act on.

## spec/violation-report/all

Every violated constraint. Needs a collection inside an error value.
