---
created-at: 2026-10-25
status: resolved
kind: feature
priority: 815
summary: "no registered queries or transactions, and no principals or definer rights in process"
closed: 2026-10-27
depends-on:
  - in-process-api
tags:
  - engine
---

# Feature: registered queries and transactions with definer rights

## Report

Implement `docs/spec/applications.md` §1, §2 and §7 and
`docs/spec/security.md` §1 and §4: registered queries and transactions
run with definer rights, principals in process, the ad hoc role.

Prerequisite: `in-process-api.md`.

## 2026-10-26: done

Landed in `crates/adtb/src/api/registered.rs`: `Registration` (the imports and items of a bundle file without its header), registered by open through `OpenOptions::registrations`, all or none, after its migrations apply; `Database::registered`, `Registered::call` and `AdHocTransaction::call` by ordinal (`import fn` first, then queries and transactions in item order). Every query, transaction, imported function and bundle function is checked and normalised at registration; a refusal fails open with `RegistrationRefused` naming the callable. `caller` reaches every statement: `Service { name: "owner" }` for the handle, else `CallOptions::assert` or `TxnOptions::assert`; asserting `Service` is `Denied`. Ad hoc text runs for the same principal. A `Principal` declared by a first migration now resolves (`evolve/resolved.rs`). Contract in `docs/design/server.md` §4. Tests: `crates/adtb-sim/tests/registered.rs` (29, all pass).

Open: registrations are not persisted to `catalog::bundles` or `catalog::registered`; no pin or translation, so a registration is checked at the version open leaves (bundles); plans are made per call, not stored at registration (`plan-cache`); argument values are not checked against their parameter types; a declared `limit` refuses registration; feeds and bundle aggregates refuse registration; no role other than the schema owner exists in process, so the grant checks of `security.md` §4.2 have nothing to check; the schema-owner role's name `owner` is unspecified.

## 2026-10-27: resolved

## 2026-10-27: landed

Landed with the regcalls track. Open items moved to `bundles`, `plan-cache`, `registered-calls-follow-ups`, `roles-and-grants` and `spec-owner-role-name`.
