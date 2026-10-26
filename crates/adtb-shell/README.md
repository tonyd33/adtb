# adtb-shell

A command-line shell over the in-process API: it opens a database file and
runs ad hoc text against it, printing each result row in the textual syntax.

Build it with `cargo build -p adtb-shell`; the binary is `target/debug/adtb`.

```
adtb FILE                              open an existing database
adtb --create --schema SCHEMA FILE     create it, applying SCHEMA as the first migration
adtb FILE -c TEXT                      run one text and exit
```

At a terminal the shell prompts and runs each text once the parser accepts
it, showing a continuation prompt while it wants more. A blank line runs an
unfinished text so its error shows, and ends a `transact` block. With stdin
not a terminal it reads stdin as a script, where an indented line also
continues the text above it. `\begin`, `\commit` and `\rollback` hold an
interactive transaction across texts; `\q` or end of input quits. In a script
or with `-c`, the exit status is 1 if any text failed.

```
$ adtb --create --schema schema.adt app.adt
$ adtb app.adt
adtb> transact
  ...   let ann = insert users { name: "ann" }
  ...   insert identities.Google { user: key ann, gid: "g-ann" }
  ...
adtb> from i in identities select match i | EmailPass e -> e.email | Google g -> g.gid | Apple a -> a.sub
"g-ann"
adtb> \begin
adtb> insert users { name: 7 }
Rejected { reason: Invalid { detail: "expected Text, found I64" } }
adtb> \rollback
adtb> \q
```
