---
title: Role drop peer mapping lifecycle evidence
---
Repository evidence at main06676b27 and unlanded role-authority provider draft,
not an external authentication-system comparison:

- docs/spec/security.md2.1 lines104-143 defines durable roles, credentials and
  drop refusal while log grants name the role or it owns a live bundle/family.
  It does not decide dependent peer mapping removal versus refusal or retention.
- security.md2.2 lines144-188 maps a certificate subject or Unix user to a role
  name by the first matching listener rule. Roles are checked after trusted
  authentication by the engine-owned role-session boundary.
- docs/spec/catalog.md2.6 lines636-671 stores immutable roles.id but peer_mappings
  has (listener, ordinal, pattern, role:Text), so the mapping has no stored target
  role-id that could distinguish deletion/recreation.
- docs/design/server.md3.1 lines152-174 requires role existence/state checks and
  opaque engine authority distinct from a supplied asserted Service value.
- Unlanded api/server/mod.rs drop_role removes the role and related grants/
  credentials while not reading peer_mappings. Keeping an old row plus creating
  the same name can therefore select the new role via an old mapping. This is a
  static witness; role successor will retain a canonical-row regression before
  dependent implementation. No working network authentication is claimed.

Example pre-state: mapping(local,1,"deploy","web") plus role(id=old,"web").
After drop/create, retaining the mapping changes its effective target to role
(id=new,"web"). Removing it atomically requires an explicit fresh map operation;
refusing drop requires explicit unmap before deleting the old identity.
