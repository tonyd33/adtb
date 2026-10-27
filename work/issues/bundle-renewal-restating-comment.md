---
created-at: 2026-10-27
status: open
kind: bug
priority: 899
summary: the new renewal comment restates its is_ok branch instead of stating a non-obvious invariant
tags:
  - quality
  - engine
---

# The renewal comment restates the branch below it

## Report

At `c317ef9a`, `crates/adtb/src/api/bundle.rs:729` adds `// A renewal that fails leaves the lease as it was, for a later call to renew.` immediately above the `if self.replace_bundle(&row, &next).is_ok()` branch. The branch itself says that the in-memory row changes only on success; the comment supplies no non-obvious invariant, and appends a rationale about a later call.

`work/tools/house-check . -r 'reviewed..c317ef9a'` reports this as the range's only added ordinary Rust comment. Remove the comment under the house rule that ordinary comments must state a non-obvious invariant. No code behaviour needs to change.
