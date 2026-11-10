---
created-at: 2026-11-10
status: open
kind: bug
priority: 500
summary: "Vm::top and Vm::extend loop forever when the map root is out of shape and the directory root no longer names it, since both reread the root fixed at open"
tags:
  - quality
  - engine
  - class-failure-flattened
---

# Bug: Vm::top and extend can spin forever

## Report

`Vm::top` (crates/adtb/src/heap/vm.rs:151-160) reads `self.root`; when it is misshapen it calls `self.refuse(.., self.named_by, shape, self.root)`, and on `None` (the directory root no longer names that map root) it loops and reads the same `self.root` again. `extend` (vm.rs:309-315) `continue`s the same way. `root` and `named_by` are fixed when the `Vm` is opened, so nothing changes between rounds and the loop never ends. dir.rs's `node()` instead returns `Ok(None)` and its caller walks again from a fresh read of the root.

Reachable only when the directory root is rewritten while a `Vm` is held, such as a drop and reuse of the relation's pages: latent.

Fix direction: on `None`, return a signal for `Heap::vm` to read the directory root's map field again, or fail as changed.

Found in the quality review of `reviewed..main` at 7bf40103.
