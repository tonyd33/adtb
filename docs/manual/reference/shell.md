# The shell

`adtb` is the command-line shell. It opens one database file, applies
the migrations you give it, and runs ad hoc text against the database:
queries, writes, `transact` blocks and the commands listed at the end
of this page.

```text
Usage: adtb [OPTIONS] <FILE>
       adtb <COMMAND>
```

The examples on this page use this schema, saved as `shelter.adt`:

```adt shelter.adt
module shelter

type Pet { name: Text } = Cat { lives: I64 } | Dog { good: Bool }
relation pets : Pet key name
```

## Flags

| Flag | Meaning |
| --- | --- |
| `FILE` | The database file to open. Exactly one is required. |
| `--create` | Create `FILE` when it does not exist. Without it, a missing file is an error. |
| `--schema FILE` | The next migration of the database's log. Repeat it once per migration, in order. The first may be a schema file instead of a migration file. |
| `-c TEXT` | Run `TEXT` and exit. Without it the shell reads texts from standard input. |
| `-h`, `--help` | Print the usage and the flags, and exit. |

Flags may come before or after `FILE`. An unknown flag, a missing
`FILE` or a second `FILE` prints the reason and the usage line and
exits with status 2.

## Creating and opening a database

`--create` makes a new database: the file itself and a directory beside
it named after the file with `-log` appended, here `shelter.db-log`.
Keep the two together; the database is both. When the file already
exists, `--create` opens it as if the flag were absent.

<!-- exit 0 1 -->
```console
$ adtb --create --schema shelter.adt shelter.db -c 'insert pets.Cat { name: "tom", lives: 9 }'
$ adtb missing.db -c 'from p in pets select p'
missing.db: the file does not exist
```

When creating fails, for example because the schema is refused, the
shell says why and removes the files it made, so you can fix the
schema and run the same command again.

Without `-c`, `--create` reads texts from standard input like any other
run: at a terminal it shows the prompt and waits. Type `\q` to leave.

This matters only for a database that a Rust program also opens. A
program registers bundles, its compiled queries, when it opens a file,
and each open marks every bundle it is not given as superseded: no
longer in use. The shell opens a file with no bundles, so after you
open a program's database in the shell, its bundles stay superseded
until the program opens the file again, which it does at its next
start. The data is unchanged. [The Rust API](rust-api.md#supersede-and-retire)
explains bundles and supersession.

## The migration list

The database keeps every migration ever applied to it, in order: its
log. `--schema` gives the shell that list from the start. The shell
compares the list with the database's log, applies the migrations the
database has not seen yet, each as one schema change, and then runs
your text. A list shorter than the log is fine, and so is no list at
all: once a migration is applied, later runs need no `--schema`.

The first file may be a schema file, a module header followed by
declarations, which the shell turns into the first migration. Every
later file must be a migration file: a module header followed by schema
operators such as `add field`.

```adt age.adt
module shelter
add field Pet.Cat.age: I64 fill 0
```

<!-- exit 0 1 -->
```console
$ adtb --schema shelter.adt --schema age.adt shelter.db -c 'from c in pets.Cat select c'
Cat { name: "tom", lives: 9, age: 0 }
$ adtb --schema age.adt shelter.db -c 'from c in pets.Cat select c'
shelter.db: the file's migration 1 differs from the log's
```

The second run lists `age.adt` first, where the log has the schema, so
the shell refuses to open the database. A migration the engine refuses
is reported by its position in the log, `migration 3 is refused:`,
followed by the reasons, and nothing of it is applied. A migration that
is applied stays applied even when the text after it fails.

The rules for applying migrations are in
[the specification](../../spec/evolution.md#2-versions).

## Running text

A text is one or more items: queries, writes, `transact` blocks and
commands. With `-c`, the shell runs one text as one transaction and
exits. The items run in order; the first one that fails ends the text,
and nothing it did commits.

<!-- exit 1 0 -->
```console
$ adtb shelter.db -c 'insert pets.Dog { name: "rex", good: true }
insert pets.Dog { name: "tom", good: false }'
`shelter::pets` already holds a row with key `pets.key("tom")`
$ adtb shelter.db -c 'from p in pets select p.name'
"tom"
```

The second insert collides with the cat `tom`, so the first insert,
`rex`, does not commit either.

Without `-c`, the shell reads texts from standard input. Each text runs
as a transaction of its own, unless a `\begin` has opened one.

### Scripts

When standard input is a file or a pipe, as in
`adtb shelter.db < script.adt`, the shell reads it as a script:

- a blank line ends a text;
- a line starting with `--` is a comment;
- a line indented with a space or a tab continues the text before it;
- any other line continues the text before it only while that text is
  unfinished, and otherwise starts a new text.

So in this script the `order by` line is indented, because
`from p in pets select p.name` is already a whole query:

```adt report.adt
-- every pet, by name
from p in pets
  select p.name
  order by p.name

count(p for p in pets)
```

```console
$ adtb shelter.db < report.adt
"tom"
1
```

A text that fails prints its error and the script goes on with the
next text.

### The prompt

When standard input is a terminal, the shell prompts with `adtb> `. A
text runs as soon as the lines typed so far parse as a whole text. While
they do not, the shell prompts with `  ... ` for more:

```console
$ adtb shelter.db
adtb> from p in pets
  ... select p.name
"tom"
adtb> \q
```

Two consequences follow. A `match` runs as soon as its first arm is
typed, since one arm already parses, so type a `match` on one line. A
`transact` block runs at the next blank line, since its items could
otherwise go on. A blank line also runs whatever has been typed so far.

### Backslash commands

A line holding one of these is a command to the shell itself, at the
prompt and in scripts:

| Command | Meaning |
| --- | --- |
| `\begin` | Open a transaction. The texts after it run in it until `\commit` or `\rollback`. |
| `\commit` | Commit the open transaction. |
| `\rollback` | Roll back the open transaction. |
| `\help`, `\?` | List the commands. |
| `\q` | Quit. |

A text that fails inside a transaction prints its error and leaves the
transaction open. When the input ends, or `\q` quits, with a
transaction still open, the shell rolls it back and says so:

<!-- exit 1 -->
```console
$ adtb shelter.db
adtb> \begin
adtb> insert pets.Dog { name: "rex", good: true }
adtb> from p in pets select p.name order by p.name
"rex"
"tom"
adtb> \commit
adtb> \commit
no transaction is open
adtb> \begin
adtb> insert pets.Dog { name: "fido", good: false }
adtb> \q
the open transaction is rolled back
```

## What the shell prints

A query prints each element of its result on a line of its own, in the
textual value syntax: the syntax you write values in. A value of a
type with no literal, such as `Date` or `Instant`, prints in the form
`T.parse` reads ([the prelude](prelude.md#values-with-no-literal)). Type it back as
`Date.parse("2026-10-25")`, not as `2026-10-25`, which is integer
subtraction. A write prints
nothing, and a `transact` block prints its `return` value, if any. A
query that is not a set, such as a count, prints its one value.

```console
$ adtb shelter.db
adtb> from p in pets select p order by p.name
Dog { name: "rex", good: True }
Cat { name: "tom", lives: 9, age: 0 }
adtb> from p in pets select { name: p.name, kind: tag(p) } order by p.name
{ kind: Dog, name: "rex" }
{ kind: Cat, name: "tom" }
adtb> count(p for p in pets)
2
adtb> max(c.lives for c in pets.Cat)
Some { value: 9 }
adtb> \q
```

A constructor value prints its fields in declaration order; an
anonymous record prints them in name order. Without `order by` the
order rows print in is not promised.

Errors print on standard error, one per failed text. In the examples on
these pages they appear where they happen.

## Exit status

| Status | When |
| --- | --- |
| 0 | Every text and command succeeded, or the shell ran at a prompt. |
| 1 | A text or command failed, or the database could not be opened or migrated. |
| 1 | `adtb verify FILE` found something. |
| 2 | `adtb verify FILE` could not run: the file refuses to open, or another process holds it. |
| 2 | The arguments were wrong. |

`adtb verify FILE` opens `FILE` as the one process holding it and checks
it whole, the allocation bitmap and the heaps included, which `verify`
at a prompt cannot do while writers may run. It prints a row per finding
and nothing for a sound file. `--schema` works as above.

At a prompt the shell exits with 0 even when a text failed, since you
saw the error.

## Parameters

In ad hoc text a free lowercase name is a parameter. The shell supplies
no parameters, so a text that uses one is refused:

<!-- exit 1 -->
```console
$ adtb shelter.db -c 'from p in pets where p.name == who select p'
`who` is a parameter of this text, but the call supplies no value for it
```

Write the value into the text instead.

## Commands

These commands run in the shell as ad hoc text:

| Command | Effect |
| --- | --- |
| `explain Q` | Prints the plan of query or write `Q` without running it. |
| `explain analyze Q` | Runs `Q` inside the statement, discards its effects, and prints the plan with the rows and time each part took. |
| `verify`, `verify R` | Checks the whole database, or relation `R`, while it serves calls, and prints a row of `sys::verify_findings` per finding. The rows stay after a reopen; a run replaces only those of the last run over its scope. It does not run inside a transaction or in text that writes. |
| `reindex R` | Starts a job rebuilding every index of relation `R`, and prints its key in `sys::jobs`. `reindex R index (f)` rebuilds one, `in S` naming it among several with those fields; `offline` makes writes to its constructors wait. |
| `vacuum R` | Removes old row versions of relation `R` that no transaction can still see, and prints, per constructor, how many it removed. |
| `set setting S = V` | Changes setting `S` and prints its new value. |

`explain` prints one record with two fields: `document`, the plan as a
JSON text for programs, and `text`, the same plan indented for people.

```console
$ adtb shelter.db
adtb> explain from c in pets.Cat select c.name
{ document: "{\"format\": 1, \"release\": \"0.1.0\", \"root\": {\"kind\": \"Set\", \"detail\": \"\", \"estimate\": null, \"children\": [{\"kind\": \"Pipeline\", \"detail\": \"\", \"estimate\": null, \"children\": [{\"kind\": \"Scan\", \"detail\": \"pets.Cat\", \"estimate\": null, \"children\": [], \"pruned\": []}, {\"kind\": \"Emit\", \"detail\": \"\", \"estimate\": null, \"children\": [], \"pruned\": []}], \"pruned\": []}], \"pruned\": []}}", text: "Set\n  Pipeline\n    Scan pets.Cat\n    Emit\n" }
adtb> set setting VacuumBudget = { io: 0.2, cpu: 0.5 }
VacuumBudget { id: settings.key(207), io: 0.2, cpu: 0.5 }
adtb> \q
```

`retire bundle B` runs too, on a bundle a program registered
([the Rust API](rust-api.md#supersede-and-retire)). `supersede bundle`
needs a live bundle to supersede by, and the shell has none. The
specification lists more commands, among them `print schema`,
`export`, `dump` and `register bundle`. The engine
does not run them yet and answers, for example,
`` `print schema` does not run in this engine yet ``. The commands and
their privileges are in
[the specification](../../spec/catalog.md#9-operational-commands).
