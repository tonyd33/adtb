---
extends: current
proposed: true
accept:
  spec/uuid-domain-format/lossless: {title: Use generic newtype text outside the sixteen-byte UUID domain, reason: 'Preserve write-only newtype domains, existing sixteen-byte UUID presentation and total lossless formatting by reusing the generic newtype spelling for other lengths'}
---
