---
created-at: 2026-10-27
status: open
kind: feature
priority: 830
summary: "no roles or grants exist, so the registration privilege check and every grant check have nothing to check"
tags:
  - engine
---

# Feature: roles and grants

## Report

`security.md` sections 2 and 3 define roles, grants and their checks, and section 4.2 requires a registering principal to hold the privileges its callables use. The engine has only the schema owner (`Service { name: "owner" }`), so registered calls run with every privilege and no grant is checked. In process this is invisible; the server needs it.

Fix: roles and grants in the catalog, the privilege check at registration, and grant checks on ad hoc text for a principal other than the owner. Then write the grant tests the regcalls track could not: a registration refused for a missing privilege, and a call by a principal without the grant.

## 2026-10-27: bundles

Also: registration privileges (applications.md check 5), retire and supersede privileges, and family owners are not enforced; every bundle is owned by "owner".
