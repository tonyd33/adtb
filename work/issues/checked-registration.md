---
created-at: 2026-10-29
status: in-progress
kind: feature
priority: 825
summary: "no role but the schema owner can register a bundle, so no grant is checked at registration and no live bundle is rechecked when a migration revokes what its owner used"
branch: checked-registration
tags:
  - engine
---

# Feature: Checked registration

## Request

Slice 3 of roles-and-grants, with the live rechecks of slice 2 folded in
(work/briefs/report-authority-next-audit.md items 1 and 2). Every bundle is
owned by the schema owner, who holds every relation grant, so a recheck on
migration or revoke can block nothing until another role can register.

Spec: security.md 2.1 (role drop), 3.5, 3.6, 4.1, 4.2, 4.6;
applications.md 3.1 (family owner), 4.1 (checks 5 and 7), 6.

## Proposal

- A trusted registration entry point taking a real RoleSession registers a
  bundle as that role. Checks 1 to 7 of applications.md 4.1 decide it as one,
  and a refusal changes nothing.
- Check 5: the role holds, with grant option, every requirement of what the
  bundle runs, using the permission provider over the Log grants of
  catalog::grants. Dormant grants allow nothing. The refusal lists every
  missing grant, in a deterministic order. The schema owner holds everything.
- The bundle's owner is the registering role. The first successful
  registration in a family commits a durable family owner with the role's
  immutable id, in the same transaction. A zero-callable bundle counts. A
  failed or unchecked header claims nothing. Check 7 refuses another role.
- Each migration rechecks every live bundle whose owner is not the schema
  owner against the grants as they stand after it, including grant option.
  This covers bundles in catalog::bundles not presented at this open. The
  refusal names every blocking bundle and changes nothing. Retired,
  superseded and expired bundles release the block.
- drop role refuses while the role owns a family or a bundle that is not
  retired.

## Lead rulings (2026-10-29)

1. The requirements checked are those of what the bundle runs: its queries
   and transactions translated to the version being checked, at registration
   and at each recheck. Pin-time metadata is never substituted for a
   translation.
2. Legacy files: a family with a header recorded before the durable family
   owner existed is owned by the schema owner. Nothing infers a successful
   registration from a header. Only the schema owner can register into such a
   family, as before.
3. The BlockingMigration notice waits for the binding slice: no session is
   bound to a bundle yet. Until then, the refusal report is the observable.
4. No owner fallback. A nonowner registration never runs with the schema
   owner's grants.

## Open questions

## Plan rulings (2026-10-29)

Plan in work/briefs/plan-checked-registration.md (local): live entry point
`RoleAuthority::register(session, registration)`, internal roots rows FAMILY,
REGISTERED (origin Open or Command, full text) and DEPLOYED, open supersedes only
Open-origin and unmarked bundles (embedding.md open step 6), Command bundles rebuilt
from stored text and rechecked at each migration.

5. A role registering identical content over a failed, unmarked header recorded by
   an open claims it and becomes the row's owner.
6. An open presenting a Command bundle keeps its owner and origin and checks that
   role's grants.
7. File reads are not a registration requirement: security.md 4.2 lists relation
   grants only, and a registered query's file reads are checked against its owner
   when they run (security.md 3.2 table, read_files). revoke_capability is not
   rechecked against bundles in this slice.
8. A grant blocker leaves `SchemaRefused.operator` empty; the detail names it.
9. The trusted API needs no capability beyond the grants (catalog.md 9.1: "as
   registration requires").

## Review round 1 (2026-10-29)

Review work/briefs/review-checked-registration.md (local): two blockers. B1: the
schema owner re-registering an expired bundle of another role skipped check 5 and
kept that role as owner; the next open then refused for good. B2: every open renewed
the lease of every Command bundle it did not present. Both go to round 3 with N1, N4,
N5, N7 and N8.

10. An `import fn` entry adds no requirement to check 5 or the recheck: security.md
    4.7 runs it with the schema owner's grants, and its `call fn` authority belongs to
    the call-grant slice. Schema functions a query or transaction reaches still count.

Follow-ups filed: open-command-bundle-failure (N2), audit-deployment-events (N3),
registered-text-retention (N6). The option-only revoke (N9) is recorded in
spec-grant-and-default-forms.
