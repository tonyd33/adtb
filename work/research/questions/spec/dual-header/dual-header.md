---
title: "How the file header survives a torn write"
pick: one
options:
  two-copies:
    title: "Two alternating copies on pages 0 and 1 with sequence and checksum"
  in-place:
    title: "One header rewritten in place"
---
Raised by the second review of the spec in docs/spec/.

## spec/dual-header/two-copies

Two alternating copies on pages 0 and 1 with sequence and checksum. A torn header would lose the checkpoint and, for encrypted files, every key.

## spec/dual-header/in-place

One header rewritten in place. Torn write loses keys.
