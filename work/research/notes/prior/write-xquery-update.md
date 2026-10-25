---
title: XQuery Update Facility pending update lists
---
Updating expressions produce an unordered pending update list; the
whole snapshot reads a fixed instance and updates apply at its end.
Conflicting primitives on one node are errors (XUDY0016 for two
replaces). https://www.w3.org/TR/xquery-update-10/ sec. 2.1, 3.2
