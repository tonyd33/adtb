---
title: "What a failing item of multi-item ad hoc text does"
pick: one
options:
  ends-text:
    title: "The first failing item ends the text; nothing commits; later items do not run; the error names its ordinal; inside an interactive transaction none of the text applies and the transaction stays open"
  per-item:
    title: "Later items still run"
---
Raised by the third review of the spec in docs/spec/.

## spec/adhoc-failure/ends-text

The first failing item ends the text; nothing commits; later items do not run; the error names its ordinal; inside an interactive transaction none of the text applies and the transaction stays open. Matches one transaction per text.

## spec/adhoc-failure/per-item

Later items still run. Partial effects in one transaction.
