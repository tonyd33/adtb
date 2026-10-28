---
title: What dropping a role does to its peer mappings
pick: one
raised-by: [spec/peer-mappings/relation]
options:
  remove:
    title: Delete mappings to the role in the same atomic role drop
  refuse:
    title: Refuse role drop while any peer mapping names it
  retain:
    title: Retain peer mappings as configuration for a role name across recreation
---
A peer mapping names a role by Text, while a role session is scoped to an
immutable role identity. The specification defines mapping order and role-drop
blockers but does not say what drop does to mappings. The choice determines
whether external credentials configured for a deleted identity can later
authenticate to a new identity with the same name.

```adt
create role web
map peer "deploy" on local to web
drop role web
create role web
```

## security/role-drop-peers/remove

Delete every catalog::peer_mappings row whose role names the dropped role as
part of the same atomic change that removes the role, credentials and applicable
deployment grants. Leave mappings to other roles and their ordinals unchanged.
A refused drop changes none of them. Recreating the name requires explicit new
mapping before the old external identity can use peer authentication again.

The maintainer recommends this. Authentication configuration belongs to the
removed role's lifecycle; keeping the catalog's Text representation does not
require name reuse to inherit trust. The cost is deliberate reprovisioning when
an administrator intentionally replaces a role but wants the same mapping.

## security/role-drop-peers/refuse

Refuse drop while any peer mapping names the role, naming the dependent mappings.
The administrator must unmap them before dropping it. This makes the dependency
explicit and preserves configuration, at the cost of an extra removal sequence
and another role-drop blocker beyond log grants and family ownership.

## security/role-drop-peers/retain

Keep mappings after role deletion. Authentication fails while the role does not
exist, then uses a recreated role with that name automatically. This treats the
mapping as a policy for a reusable name. It preserves provisioning convenience,
but external credentials configured for the old role can reach the replacement
without a new mapping decision; role recreation must account for that inherited
trust even though old opaque role sessions are invalid.
