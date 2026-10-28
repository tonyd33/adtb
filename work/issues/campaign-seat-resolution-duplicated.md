---
created-at: 2026-10-28
status: open
kind: bug
priority: 899
summary: player seat lookup and revoked-seat handling are copied across four request handlers
tags:
  - quality
---

# Player requests repeat the seat revocation lookup four times

## Report

At reviewed head e4d7aa6c, `apps/campaign/src/web/player.rs:55`, `:78`,
`:103` and `:126` contain the same canonical `v1_6::seat` call, row/error
handling and revoked-seat `401` conversion. The new Now and Recaps handlers
copy the existing Home and Entity authentication/lifecycle rule. Changes to
seat validity or refusal handling must therefore be repeated in four places.

`auth.rs` parses the cookie but provides no seat resolver; searching web/ for
`seat_token`, `v1_6::seat` and `This seat has been revoked` finds these copies.
The existing quality-refactors ticket enumerates different engine helpers.
No authorization bypass is demonstrated by this finding.

Extract one private synchronous seat resolver used inside all four blocking
closures. Keep a canonical lookup on every request, preserve exact revoked-seat
401 behavior, and avoid caching authority. Retain existing revoked-seat coverage
for Home/Entity and Now/Recaps, valid-party isolation, and successful page flows.
