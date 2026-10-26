---
extends: current
proposed: true
accept:
  spec/regex-classes/unicode: {title: 'Unicode: \d, \w, \s and \b follow the recorded Unicode version', reason: 'the classes are Unicode, following the database''s recorded Unicode version, since matches works over Unicode scalar values'}
  spec/script-format/indented: {title: 'Directives in the first column, bodies indented two spaces, rows after ---- up to a blank line, comment or directive; sets in printed order unless rowsort', reason: 'the layout the conformance.md example already uses, with the ends of bodies and rows and the meaning of query without rowsort stated'}
---
