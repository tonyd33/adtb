---
created-at: 2026-10-31
status: open
kind: bug
priority: 442
summary: Prove by generated texts that a role cannot tell an ungranted name from an absent one
---

# Adhoc name oracle property

## Report

Four rounds of review of the ad hoc slice found name oracles by hand, each with its own pair in the test list. Replace that with a property: generate texts over the grammar's statement kinds, render each twice with a name swapped between an ungranted relation, field, constructor or module and an absent one, and require the same answer and audit row for a role. Run it for a role with `adhoc` only and with each capability. It needs the front end fuzzer's generator. Start after roles-adhoc lands.
