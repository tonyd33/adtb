---
title: "How printed text writes control characters"
pick: one
options:
  unicode-escape:
    title: "Printing writes a control character as \\u{..}; reading accepts it raw or escaped"
  raw:
    title: "Printing writes every character but quote, backslash and line breaks raw"
---
Raised by the reference interpreter's printer. interop.md §1.1 said
text prints "with the escapes of syntax.md", without saying which
characters are escaped.

## spec/text-escapes/unicode-escape

Printing escapes `"` and `\`, writes a line feed, carriage return and
tab as `\n`, `\r` and `\t`, and every other control character
(general category Cc) as `\u{..}` in lowercase hex. A reader accepts
a control character written raw or escaped.

```adt fragment
"a\u{7}b"   -- printed form of a, BEL, b
```

Printed text then holds no invisible characters, and a CSV or log line
stays one line.

## spec/text-escapes/raw

Only what the grammar forces is escaped. Shorter, but a printed value
can hold a BEL, an escape sequence or a NUL, which terminals and
tools act on.
