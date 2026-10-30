---
extends: current
proposed: true
accept:
  spec/admission-bytes/held: {title: 'Arguments, ad hoc text and parameters, and a fixed per-call overhead, as admitted; Append batches after admission are not counted', reason: 'lead ruling G17: accepted as written'}
  spec/default-scope/named: {title: A default applies only to the constructors its log entry names; a new add default covers a later constructor, reason: 'lead ruling G14: a default applies only to the constructors its log entry names'}
  spec/empty-carry/drop: {title: The migration drops it, reason: lead ruling N3; the code keeps the row and is filed}
  spec/family-grant-row/latest: {title: 'One row per grantee, family and query with the latest signature; a regrant replaces it', reason: 'lead ruling G9: one row per grantee, family and query, replaced by a regrant'}
  spec/family-revoke/widen: {title: 'It matches the family row whatever its signature; one row per grantee, family, query and grantor', reason: lead ruling N1}
  spec/family-signature-version/at-version: {title: The text names the log version at which its type names resolve; equality is by the ids those names had then, reason: lead ruling N2; notation chosen by the spec writer}
  spec/grant-identity/target-text: {title: 'A grant''s target text is its identity; grant and revoke accept that text, and a revoke matches it exactly', reason: 'lead ruling G1 and G4: the target text is the identity, so a carried grant can always be revoked'}
  spec/inprocess-admission/same: {title: Every in-process call and ad hoc text passes the admission a server call does, reason: 'lead ruling G18: the spec stands; the synchronous path is a code gap'}
  spec/operator-grantor/schema-owner: {title: The schema owner, reason: 'lead ruling G7: an operator grant is given by the schema owner'}
  spec/operator-revoke/schema-owner: {title: 'As the schema owner, the grantor of every operator grant', reason: lead ruling R1}
  spec/restore-owner/file: {title: 'The file''s owner role; the dump''s owner row maps to it, and a dump naming the owner differently is refused', reason: 'lead ruling G19: a restore keeps the file schema-owner role'}
  spec/revoke-grantor/per-grantor: {title: 'Rows are keyed by grantee, privilege, target and grantor; a revoke deletes the issuer''s rows, the schema owner''s every grantor''s, and refuses to orphan rows a deleted option supported', reason: 'lead ruling G7: a revoke never silently removes another grantor or orphans supported rows'}
  spec/revoke-normal/row-form: {title: 'Resolve the revoke against the schema and print it in row form, then compare exactly', reason: lead ruling R2}
  spec/revoke-widening/keep: {title: A revoke without a field list matches every frozen field set; a retag revoke without except matches whatever pairs are excepted, reason: 'lead ruling: both widenings confirmed'}
  spec/signature-equality/ids: {title: Each named type by type id with its constructor and field ids; anonymous types structurally over the same, reason: 'lead ruling G8: any difference in type, constructor or field ids leaves a bundle uncovered'}
  spec/target-order/ids: {title: 'Id order, as fields', reason: lead ruling N4}
---
