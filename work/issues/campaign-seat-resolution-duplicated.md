---
created-at: 2026-10-28
status: resolved
kind: bug
priority: 899
summary: player seat lookup and revoked-seat handling are copied across four request handlers
closed: 2026-10-28
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

## 2026-10-28: resolved

Landed87527a906c72383b72b365a4f88ccf3c700c5986 after manual corrections.
Private generated-type resolve_seat replaces four identical calls in player.rs;
canonical lookup on each request and exact error/party/visibility behavior remain.
No authority cache, dependency, schema, public API or new mirror test. Fresh
independent reviewer cleared exact candidate2bfba389 and all2575source hashes.
Author app47pass/1existing skip; strictclippy/fmt/lintparity/house/size0.
SourceSHA283713ceab344cba7720b62bc123195095768122af8870815d2d226f0e55547f
is identical after landing. Review/receipt/handoff local work/briefs same stem.

Lead fullmain on empty6af7c3cd above87527a90: direnv exec main env
DEVENV_ROOT=$PWD CARGO_TARGET_DIR=$PWD/target check, default configured
profiles/features/toolchain. Collected exit0:2426root pass/8existing skips
(52.065s),47campaign pass/1existing skip (21.126s), strictclippy/fmt root+app,
lint parity, dependency policy, size and native docs all0. Docs10tests10.116s,
422syntax/61pages/6736links/5001fragments/0failures. Existing cargo-deny
unmatched license/skip warnings remain nonfatal. All tracked+present local
cargo-config hashes and revision unchanged. Evidence target/campaign-seat-
integration/{check.log,check.status,start.txt,end.txt,inputs-before.sha256,
inputs-after.sha256,revision-before.txt,revision-after.txt}. LogSHA256:
31a1bd50e3386e60dc906eaf405d2b7b8b1cee8734bfe1aec60ef5565e916145
Measured UTC boundaries start/end retained; queue/permission breakdown unknown.
No engine failure found in this cleanup. Shared fix-row-provenance prerequisite
arose independently in unlanded permissions tests and remains in-progress.

## 2026-10-28: resolved
