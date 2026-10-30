---
title: "The name of the command line tool"
pick: one
options:
  adtb:
    title: "adtb, as built and as the manual says"
  adt:
    title: "adt, as the spec said"
---
Raised by the milestone 9 audit. The spec's examples run `adt verify app.adt`; the binary and the signed-off manual run `adtb`. The file extension `.adt` and the `adt://` scheme are a different question and stay.

## spec/tool-name/adtb

Rename in the spec's examples. The manual is signed off and users already type it.

## spec/tool-name/adt

Rename the binary and rewrite the signed-off manual, for one fewer letter.
