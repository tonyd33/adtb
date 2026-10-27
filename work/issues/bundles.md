---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 820
summary: "no bundles: families, registration checks, pinning, translation, leases and supersession do not exist"
closed: 2026-10-27
depends-on:
  - operator-log
  - write-states
  - registered-calls
tags:
  - engine
---

# Feature: bundles and families

## Report

Implement `docs/spec/applications.md` §3 to §6, §8 and §9 and
`docs/spec/evolution.md` §7: bundles and families, registration
checks, pinning, translation of pinned queries, leases, supersession
at open, `hide new constructors`, the recheck of registered queries on
a schema change. Settle `schema-change.md:75`, the recheck contract.

Prerequisites: `operator-log.md`, `write-states.md`,
`registered-calls.md`.

## 2026-10-27: registrations landed

Registered calls landed without persistence: registrations live in `OpenOptions::registrations` for one open and are not written to `catalog::bundles` or `catalog::registered`, and a registration is checked at the version open leaves with no pin or translation. `Registration::new(name, text)` takes a bundle file without its header; wrap it with the header here. `docs/design/server.md` section 4 has a TODO for how a pin, translation and lease reach a registration. Start from `checked` in `crates/adtb/src/api/registered.rs`.

## 2026-10-27: done

Landed: `Registration::bundle(text)` reads a bundle file with its header (name, family and version, pin with its hash, edition, lease, priority, `hide new constructors`; a budget clause is refused). Open records the given bundles in `catalog::bundles` and supersedes every other active one in one durable commit before its migrations, refuses a downgrade whose bundle would read a constructor enabled since its pin (`DowngradeRefused`), and after the migrations checks each bundle at its pin (pin present with its hash, edition, check at the pinned source, retired name, different content, `BundleStale`, `BundleExpired` for an expired one) and registers all or none, storing its calls in `catalog::registered`. Registrations survive reopen and crash. Ordinals start at 1. Binding (`Database::registered`) and calls refuse `Retired`, `BundleSuperseded` and `BundleExpired` (lease from the header or 30 days, an hour's grace, renewal at most once an eighth of the lease, none after the lease ran out). `retire bundle` and `supersede bundle` run as commands. A bundle pinned before the current version registers and checks at its pin, and its calls are refused `Untranslatable`. Readers are derived from each bundle's normal forms (`api/reads.rs`) and decide staleness through `Catalog::stale`. The recheck contract is in `docs/design/schema-change.md` section 2 and the registration contract in `docs/design/server.md` section 4.

Open: version translation (`version-translation`), so a migration is not yet refused for a live bundle whose query has no translation; `register bundle "file"` and `apply migrations` as in-process commands, so no live reader exists while a migration runs in process; `LeaseGrace`, `DefaultLease` and `BlockingNoticePeriod` settings are constants, not read from `catalog::settings`; the bundle owner is always the schema owner and the privilege checks of registration (check 5) and of retire and supersede are not made; `catalog::bundles` has no mark for a bundle registered other than by an open, so an open supersedes every bundle it is not given; catalog pins are refused; `print schema` does not run.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the bundles track.
