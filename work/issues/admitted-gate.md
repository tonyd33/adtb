---
created-at: 2026-10-31
status: open
kind: bug
priority: 404
summary: "authority, admission and audit are checked per execution entry path"
---

# Admitted gate

## Report

Authority, admission, audit and deadline are applied per entry path, and each new path can miss one: sync-call-skips-admission, confined-refusal-unaudited, roles-adhoc rounds 1 and 3.

One type, constructible only by authorising then admitting, required by every execution entry, the sync path included. Its constructor writes the audit row and sets the deadline. Delete the per-path checks and the `commanded` special case. The socket slice lands on it: adtb-server has wire framing and no listener, so no client can connect.

Land after roles-adhoc. Dissolves sync-call-skips-admission, confined-refusal-unaudited, part of audit-follow-ups, and supports adhoc-name-oracle-property.
