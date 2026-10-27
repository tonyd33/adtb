---
created-at: 2026-10-27
status: open
kind: bug
priority: 874
summary: "adtb-probe decides agree or differ by parsing the conformance runner's message text, so a reason containing a colon or a wording change flips the verdict"
tags: [quality, test]
---

# adtb-probe's verdict depends on the wording of conformance messages

## Report

`crates/adtb-probe/src/main.rs` reads each side's outcome from adtb-conformance's printed messages: `strip_prefix("got ")` at `:243-250`, `gist` splitting at `"refused with "` and the first `:` at `:271-280`, and `plan_text` splitting at `text: \"` and `\" }]` at `:283-287`. A refusal reason containing `:`, or any change to the message format, changes the verdict without an error.

Fix: have adtb-conformance return a typed outcome per directive (rows, or a refusal with its reason) and compare those.
