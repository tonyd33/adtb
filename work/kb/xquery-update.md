# XQuery Update Facility 1.0

kind: language
home: https://www.w3.org/TR/xquery-update-10/

## Semantics

- A pending update list is an unordered collection of update primitives not yet applied; a snapshot is a scope evaluated against a fixed XDM instance with updates held pending, ended by `upd:applyUpdates`. [V 2026-10-25] https://www.w3.org/TR/xquery-update-10/ sec. 2.1, 3.2.2
- Deletions take effect only at snapshot end and do not affect variable bindings. [V 2026-10-25] same, sec. 2.4.2
- Conflicting primitives on one node raise errors at merge/apply: two renames XUDY0015, two replaceNode XUDY0016, two replaceValue XUDY0017. [V 2026-10-25] same, sec. 3.2.1
