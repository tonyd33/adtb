---
created-at: 2026-10-28
status: in-progress
kind: bug
priority: 899
summary: player seat lookup and revoked-seat handling are copied across four request handlers
branch: campaign-seat-cleanup
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

## 2026-10-28: parallel private cleanup track

User requested parallel work. Fresh campaign-seat-cleanup workspace16f4d11e on
999926a8 owns only apps/campaign/src/web/player.rs and necessary imports. Keep
canonical lookup per request, exact refusal/error/party scope and generated types;
no cache or behavior change. Parallel permissions language and manual prose tracks
have disjoint ownership and do not expose a shared signature this helper consumes.

First serialized Rust slot: app strict clippy/fmt and all campaign tests with
root nextest config, house/size and full diff review. No new mirror test needed
when existing revoked/party/isolation flows cover the refactor. Fresh independent
review and lead full main check before resolving. Root full branch suite is
intentionally replaced by app acceptance because production change is app-only.
Landing manual-derived first, this track second, permissions last; rebase source
if directed. Exact source/config/command/status/log/timing receipt required.
Local brief/receipt/handoff under work/briefs/*campaign-seat-cleanup*.
