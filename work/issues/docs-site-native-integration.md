---
created-at: 2026-10-27
status: resolved
kind: bug
priority: 898
summary: "docs site wraps mdBook with unnecessary staging, watchers and ignore overrides"
closed: 2026-10-27
tags:
  - quality
  - tooling
---

# Docs site native integration

## Report

User rejected tools/docs-site/site.py quality. Lead approved a functional repair
without challenging why a Python polling supervisor, symlink source view,
generated configuration/assets and book-local ignore override existed. Tests
proved the layers worked, not that mdBook needed them. The temporary-file atomic
publication itself is conventional, but was part of an unnecessary pipeline.

## 2026-10-28: corrective track and review bar

Fresh docs-native workspace /home/tony/code/adtb-docs-native on d157a20d, brief
brief-docs-native.md. Prototype proved native mdBook preprocessor Chapter insertion.
Approved routine baseline: checked-in book.toml, docs source directly, minimal
checked-in SUMMARY roots/landing, stateless supported README navigation preprocessor,
direct JS asset path, native mdBook build/serve/watch/lifecycle, separate checks.
Remove obsolete staging/generated config/copied sources/assets/custom watcher/
supervisor/ignore override/temp publication path. No new dependency or hosting.
Preserve unchanged authored manual/spec/design sources, navigation order/live
updates/search/highlighting and actualHTTP checks/malformed recovery/shutdown.

Lead workflow now requires architecture comparison with native configuration/
extension points and fresh review challenging each customlayer's unmet requirement;
green tests are not a design justification. Fresh independent architecture AND
correctness review, scoped docs/syntax/link/HTTP checks plus main verification gate
landing. Retain exact tested/base IDs, stable inputs, exits/logs and remaining limits.

## 2026-10-28: native candidate and independent review

Workerfinal e58482ec096867ca4d9059904715fea803173ee5/base d157a20d: site.py deleted;
checked-inbook.toml/docsSUMMARY, stateless nativeJSONBook navpreprocessor, direct
mdbookexec and separatevalidators. All removed layers remain absent. Native+wrapper
buildfrom/tmp,10Python/420syntax/61pages6730links4995fragments andhouse pass0,
13.354s, all tracked/localconfig hashes+revision unchanged. This is scope evidence,
not landing. Independent reviewer proposed nativebaseline and finds no architecture
blocker: only navderivation needscustomproduction code. Reviewer exactfinal tests/
report and reused desktop/mobile browser smoke still completing before landing.

## 2026-10-28: resolved

Landed9c52a660 refactor(docs): use native mdBook integration. Deletedsite.py and
its staging/symlinks/generatedconfig/assetcopies/temp publication/Pythonpolling/
supervisor/ignoreoverride. Nativebook.toml source=docs and directJS; minimalSUMMARY
roots and byte-identical movedlanding. Stateless nativeJSONBook preprocessor only
deriveschapters from existingREADME indexes. Thin rootselectingCLIexecs mdbook;
independent syntax/linkchecks remain validation, not presentation runtime. Native
commands work directly. No newdeps/hosting/authored manual/spec/design rewrites.

Fresh independent architecture review of exacte58482ec096867ca4d9059904715fea803173ee5
ond157a20d6d714916f9afce9576f6856458dd102a compared the zero-code staticSUMMARY
alternative: it duplicates navigation or drops accepted derivedlive navigation.
Preprocessor is the one justified customproduction extension; mdBook owns
rendering/assets/discovery/watch/lifecycle. Reviewer read allfiles/deltas, native
build from/tmp and own exactdocs/house checks passed; report in work/briefs.
This rationale, not passingtests, is the architecture clearance. Leadworkflow
changed to require nativebaseline and fresh responsibility/layer review before
accepting customorchestration; former lead review had missed that challenge.

Workerfrozen scoped4commands0/stablehashes13.354s: directnative+wrapperbuildfrom/tmp,
10Python10.125s,420syntax61pages6730links4995fragments0fail,house0. Browser desktop/
mobile/theme/ADTspans/crosssectionlinks/numericanchors/allsectionsearch/nooverflow/
zeroconsoleerrors smoke0,3.202s; serverreapedSIGTERM-15. Initial bad-unhashedasset
oraclefailure retained and corrected to actualnativehashed references, no aliascopy.

Leadreviewed rebaseddelta: only workflow/ticketmetadata since testedbranch; all
book/doc/tool inputs identical. Leadscopedmaincheck0 at cleand5ec04838a5d76f82b87111580175d8ee5bf9601
parent9c52a660,2026-10-28T04:57:25.536452Z–04:57:37.521932Z,11.986s. Same10tests/
420syntax61pages6730links4995fragments0failure; alltracked/localconfig hashes and
revision unchanged. Command mainroot: direnv exec . env DEVENV_ROOT=/home/tony/code/adtb
CARGO_TARGET_DIR=/home/tony/code/adtb/target python3 -B target/check-docs-native-main.py
Runner calls work/tools/docs-site check. LogSHA256b82b221dbe1260e996ca07db802a62919be327c87ae474d9e41faaaa1ce06d38.
Python3.14.7/mdBook0.5.4/existingQuickJS/defaultbookconfig; permission/queue/execution-only
unknown. Logs target/docs-native-main-check/; workerreceipt/handoff-docs-native.md.
Obsolete main generatedsource/config/served artifacts removed after retaining
smalllegacyconfig evidence; all earlierfailure/review/browser evidence retained.
RootRust checks deliberately deferred to lead combinedfullcheck with pending
system-write-guard (no Rust/builddependency change in this docs track). Noactive
servers/checks or unresolved docs failure. Hosting remains separate.

## 2026-10-27: resolved

## 2026-10-28: combined integration verified

After system-write guard `d2716fb0`, the complete main check passed with native
mdBook:2366 engine tests,46 campaign tests, all lint/dependency/size checks and
10 Python docs tests,420 syntax inputs,61 pages,6730 links,4995 fragments.
Stable tested main working copy `d8f46ebd225392b61d287cf78a5e5a51eb16dd3d`;
397s,05:25:28Z–05:32:05Z. Exact command and evidence are in
system-relations-program-writes and target/system-guard-integration. The earlier
outstanding combined Rust check is now complete.
