---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 898
summary: docs serve watches source bodies but never regenerates navigation or copied grammar
closed: 2026-10-27
tags:
  - quality
  - tooling
---

# Docs serve retains stale generated navigation

## Report

At be6b9b4a, tools/docs-site/site.py:157–161 runs prepare once and execs
mdbook serve. prepare generates SUMMARY.md and copies adt-highlight.js.
Source watching rebuilds page bodies but cannot regenerate these files.

Run work/tools/docs-site serve, add docs/manual/new.md and link it from the
manual README, or reorder that README's links. The generated SUMMARY remains
unchanged: the new page is missing from the sidebar/build and its README link
can lead to a 404 until the server restarts. Grammar edits similarly retain
the copied version. Existing watch verification exercised body edits only.

Regenerate derived navigation/assets during serving when their sources change.
Keep authored Markdown unchanged and add a regression that changes navigation
while the server remains running.

## 2026-10-28: local serving repair dispatched

Fresh docs-nav workspace ../adtb-docs-nav on f305ffaf owns only
site tooling/regression and optional development instructions. Keep mdBook,
authored Markdown and separate hosting milestone. Regenerate navigation/copied
assets during actual serving; test page add/remove/reorder, grammar/body edits,
transient invalid navigation recovery, no feedback rebuild loop and graceful
shutdown against real HTTP output. No new dependencies. Scope verification:
work/tools/docs-site check, real serve regression, house and diff review; full
Rust integration remains the lead's responsibility. Detailed ignored brief
work/briefs/brief-docs-nav.md is not the only durable scope record.

## 2026-10-28: resolved

Landed2bc9c2be after exact lead review of6cb5ae4cfb1db1c3dd487fa3a6aba4b756c44866
on f305ffaf. Serving observes authored input edits, regenerates navigation/assets
with atomic derived-file publication, preserves valid SUMMARY during incomplete
index edits, logs once and recovers; subprocess status and both stop signals
reap mdBook. Generated local empty.gitignore keeps mdBook0.5.4 portable poller
from pruning the ignored source view. Native watcher idle loop was rejected,
not shipped. No dependency/engine/spec changes; hosting remains separate.

Actual HTTP regression verifies add/remove/reorder, body/title, grammar/home,
invalid-index recovery, stable mtimes AND build count while idle, child failure,
SIGTERM/SIGINT and closed listener. Original script fails added-page HTTP witness;
logs retained. Final worker docs/house0,8tests420syntax61pages6730links4995fragments,
stable6cb5ae4c inputs,11.506s. Lead scoped main docs0 with same counts,11.084s,
at cleanb4ff5c06af7ee3f0103c503f715248f3791fda1f parent2bc9c2be; revision and all
tracked/localconfig SHA256 inputs unchanged before/after. Command main root:
direnv exec . env DEVENV_ROOT=../adtb
CARGO_TARGET_DIR=../adtb/target python3 -B target/check-docs-nav-main.py
Runner calls work/tools/docs-site check. Evidence target/docs-nav-main-check/.
Python3.14.7/mdBook0.5.4/defaultconfiguration; permission/queue/execution-only
unknown. Branch rootRust check intentionally not run; combined full check belongs
to lead after priority engine sequence repair. This is a docs-scope pass only.

## 2026-10-27: resolved
