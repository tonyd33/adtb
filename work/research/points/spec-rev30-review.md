---
extends: current
proposed: true
accept:
  spec/family-grant-row/latest: {title: 'One row per grantee, family and query with the latest signature; a regrant replaces it', reason: 'lead ruling G9: one row per grantee, family and query, replaced by a regrant'}
  spec/grant-identity/target-text: {title: 'A grant''s target text is its identity; grant and revoke accept that text, and a revoke matches it exactly', reason: 'lead ruling G1 and G4: the target text is the identity, so a carried grant can always be revoked'}
  spec/revoke-grantor/per-grantor: {title: 'Rows are keyed by grantee, privilege, target and grantor; a revoke deletes the issuer''s rows, the schema owner''s every grantor''s, and refuses to orphan rows a deleted option supported', reason: 'lead ruling G7: a revoke never silently removes another grantor or orphans supported rows'}
  spec/signature-equality/ids: {title: Each named type by type id with its constructor and field ids; anonymous types structurally over the same, reason: 'lead ruling G8: any difference in type, constructor or field ids leaves a bundle uncovered'}
---
