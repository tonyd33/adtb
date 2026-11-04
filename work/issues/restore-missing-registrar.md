---
created-at: 2026-11-04
status: open
kind: feature
priority: 600
summary: A dump whose bundle marks or family owners name a role the dump lacks is refused at restore; the spec could restore them as dormant grants are
---

# Feature: Restore of a bundle whose registering role is absent

## Request

A dump can name, in a bundle's registration mark or a family owner, a role it does not carry: a dump without `security` (interop.md 6.1 writes `security` only for a holder of `schema`), or an edited one. Restored as is, the file's next open fails re-registering the bundle (`no role has id N, which owns bundle B`). Dump S5.2 refuses such a dump at restore, naming the bundle's entry, as an interim ruling (2026-11-04). The spec could instead restore it as dormant grants are restored: the bundle kept, its registration or family owner marked for the owner to claim. That is a design question for the space before any change.

## Proposal

## Open questions
