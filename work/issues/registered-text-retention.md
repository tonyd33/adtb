---
created-at: 2026-10-29
status: open
kind: feature
priority: 300
summary: "REGISTERED roots rows keep every registration's full file text forever, including for retired bundles"
---

# Feature: Registered text retention

## Request

Found by the checked-registration review (N6). Each REGISTERED roots row holds the bundle file text so a later open can rebuild it. Nothing deletes the row once the bundle is retired, superseded or expired, so the roots heap grows with every registration. Drop the text of a released bundle, or move it out of the roots heap.
