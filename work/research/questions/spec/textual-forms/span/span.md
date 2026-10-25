---
title: "The textual form of a Span"
pick: one
raised-by: [spec/textual-forms/interop]
options:
  call:
    title: "span(a, b), the call that builds it"
  record:
    title: "The record form { start: a, end: b }"
---
Raised by the reference interpreter's printer. A `Span a` is built only
by `span` (prelude.md §2.8), and interop.md §1 gave it no form.

## spec/textual-forms/span/call

A span prints and reads as `span(a, b)`, its bounds in their own
forms:

```adt fragment
span(3, 7)
span(Instant.parse("2026-10-25T09:00:00.000000Z"), ...)
```

The form is the only expression that builds one, so the text reads
back as written; a reader refuses a start after the end as `span`
does.

## spec/textual-forms/span/record

`{ start: 3, end: 7 }`, as any record prints. But a record literal of
`Span` is refused in queries, so the text would not read as query
source, and the reader would need its own check of the bounds.
