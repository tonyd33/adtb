---
created-at: 2026-11-06
status: open
kind: feature
priority: 130
summary: "evidence about a page (damage records, LSN chain proofs, LSN map entries, verify and settle bases) is keyed by page id across a free and reallocation, so one incarnation's evidence judges another's"
---

# Feature: key page evidence by incarnation, not page id

## Request

## Proposal

## Open questions

## 2026-11-07: design ruled

Design in the local receipt receipt-page-incarnation-r1. Cause: a free logs nothing for a healthy page and an allocation continues the old chain (`reserve_page` keeps the old LSN, the first record has `prev = page::lsn`), so nothing in the log marks an incarnation's start. The read-in gate is unaffected (expected LSNs only rise); settle and evidence carried by page id are not.

Lead rulings:
- M1: an allocation overwrites the reserved page unread and logs `Image { prev: UNCHAINED }`, the chain start recovery, verify and `resolutions_from` already honour; a pid in `alloc.reserved` gets a blank frame in `resident`; the damage fold drops every entry at a chain start. Full image, not a small change over a blank page: no record or torn-page rule change, and allocations are rare beside writes.
- Every free logs a `Freed` record, so the log marks an incarnation's end as well as its start.
- M2: a status page keeps its page number (mvcc-settle-recycled). Settle is the only reader of a copy the gate refused, and the number is that copy's identity.
- The allocator may reuse a free page refused while free, except one refused Unreadable: M1 never reads it.
- PAGE_FORMAT 2 with no format-1 decoder does not break layout/format-version/read-old: it binds formats of an earlier release, and none has been cut.
- Order: M1 with `Freed`; then the class search with its failing cells noted; then the evidence authority, designed from what the search fails, through the design space, one consumer at a time; then instances the search still finds. The search comes before the authority so the authority is designed from failures, not reasoning.
- The search is the class's exit criterion. Its per-change tier (about 400 cases, 2 to 3 min wall) runs in `check --changed` when a change reaches storage (maintainer, 2026-11-07); the exhaustive tier with fault pairs and the concurrent tier runs under the deep profile.
- Ticket map: settle via mvcc-settle-recycled; verify-free-page-drops-chain and damaged-record-after-free (reallocation half) by M1 and `Freed`; catalog-settled-inprogress and unreadable-bitmap-hides-group by the authority; backup-list-misses-late-stale to be confirmed fixed by the search; incomplete-mapping-never-retried outside these, a periodic retry.
