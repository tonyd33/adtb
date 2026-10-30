---
created-at: 2026-10-29
status: open
kind: bug
priority: 780
summary: Refusing a Set Text field also prints a cascade error naming no type Member
---

# Refusing a Set Text field also prints a cascade error naming no type Member

## Report

Found by the manual reader check on 2026-10-29. The `Set Text` field refusal shown at
docs/manual/concepts/keys-and-references.md:154-156 prints a second line, "no type `Member`".
It follows from the first error, not from the text. Fix: stop resolving names that depend on a
refused field, so only the first error prints; update the manual page.
