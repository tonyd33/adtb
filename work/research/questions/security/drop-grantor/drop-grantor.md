---
title: What dropping a role does to the rows it gave
pick: one
options:
  refuse:
    title: Refuse naming the rows while the role is grantor of any deployment row
  delete:
    title: Delete them with the role
  cascade:
    title: Delete them and everything that relied on them
---
ann gives bob `monitor` with option, bob gives cid. `drop role ann`.

## security/drop-grantor/refuse

Only a row's grantor or the schema owner revokes it, and a revoke that orphans a row is refused. A drop must not be the way around either. The schema owner revokes the rows first. The rows the role holds as grantee go with it. This one rule covers rows below its grantees too, since those rely on a row it gave.

## security/drop-grantor/delete

bob silently loses `monitor` and cid's row stays with no option above it.

## security/drop-grantor/cascade

One statement silently removes access from roles it never names.
