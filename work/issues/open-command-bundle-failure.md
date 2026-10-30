---
created-at: 2026-10-29
status: open
kind: feature
priority: 610
summary: "an unpresented role-registered bundle that fails its check at open fails the whole open, and nothing can retire it without an open"
---

# Feature: Open command bundle failure

## Request

Found by the checked-registration review (N2). At open, each live bundle a role registered through a session is rebuilt from its stored text and checked. One that fails (a host function redefined, a future edition change) fails the whole open, and no API retires it without an open. embedding.md step 9 speaks only of given bundles. Design question: refuse the open naming the bundle with a way to retire it from open options, or open without serving it and report it.
