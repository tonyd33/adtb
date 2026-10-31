---
title: What reading a sys relation or catalog::plans needs
pick: one
options:
  monitor-or-read:
    title: monitor, or a read grant on that relation
  monitor-only:
    title: monitor always
---
A role holding `schema` can grant read on a system relation, and 6.2 says reading needs monitor.

## security/system-read/monitor-or-read

The grant is the narrow form of monitor. Revealing columns still need read of what they reveal.

## security/system-read/monitor-only

The grant would do nothing.
