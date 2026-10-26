---
title: "Which ids the definitions an install package brings take"
pick: one
options:
  as-operators:
    title: "As if the package's declarations were operators of the installing migration, after install, in package order"
  package-space:
    title: "Each package numbers its own objects, offset into a range the install reserves"
---
Raised by `id-assignment-gaps`. `install m "v" from "file"` adds a
module with its definitions and code (interop.md §9.6); §1.2 gives
them no ids. `uninstall` and a later `install` of another version
must not reuse them.

## spec/ids/package/as-operators

The install creates the module's objects in the package's declaration
order, as `add type`, `add relation` and `define fn` would, each
taking the next id of its space. Ids depend on the log as every other
id does.

## spec/ids/package/package-space

The package states its objects' relative ids, and the install reserves
a block of each space for them. Upgrading a package can keep ids, but
blocks fragment the spaces.
