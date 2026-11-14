---
created-at: 2026-11-14
status: open
kind: feature
priority: 200
summary: "the kernel validator leaves some section 3 and section 5 rows unchecked"
tags:
  - lang
---

# Feature: check the remaining section 3 and section 5 rows in the kernel validator

## Report

Left open by structure-lang-identity step 8a (a1436ec3).

§5 rows unchecked". What stays loose after this change:
- `Top`'s keys (each over `var`, ordered) are not checked;
- `Present`: only `τ` against the source is checked, not its placement, `take : I64` or
  `after`'s type;
- `Agg::User`: the body is not checked against the aggregate's one parameter type;
- `Restrict` over a non-sum element passes with no constructor-set check;
- `Ctor` under an anonymous `Sum`: the constructor is checked against the sum's constructors
  but not against its selection `sel`;
- `Transition` placement (only in a constraint body) is not checked;
- §5 argument types of primitives: arity and result are checked, operand types mostly not.
