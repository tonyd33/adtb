---
title: The canonical textual syntax for values
pick: one
options:
  literal:
    title: The query language's literal syntax is the text format
  json:
    title: The JSON mapping is the text format
---
Dumps, CSV cells, error messages, the REPL and test fixtures print
values. One syntax should parse back to the same value
(`Decimal 2` keeps its scale, `F64` keeps NaN payloads or states that
it does not, `Instant` keeps its precision).

## interop/text/literal

```
Google { id: 7, user: Key users 3, gid: "g-123" }
Some { value: Decimal 2 "19.90" }
```

What a user types in a query is what is printed. A parser outside the
engine needs the language's lexer.

## interop/text/json

```
{"Google": {"id": 7, "user": 3, "gid": "g-123"}}
```

Any language reads it. JSON numbers lose `I64` and `Decimal` precision
in common parsers, so exact numbers print as strings and the schema is
needed to read them back.
