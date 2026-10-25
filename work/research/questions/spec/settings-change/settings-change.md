---
title: "How settings change"
pick: one
options:
  command:
    title: "set setting X = v, needing configure"
  write:
    title: "A write to catalog::settings"
---
Raised while drafting and reviewing the spec in docs/spec/.

## spec/settings-change/command

set setting X = v, needing configure. Settings are effects, not data a registered query writes.

## spec/settings-change/write

A write to catalog::settings. No server path could perform it.
