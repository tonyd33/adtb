# Files and CSV scans

adtb reads CSV and NDJSON files as relations. You query a file the way
you query a relation, join it with your data, and load it with an
ordinary `insert`. This guide loads a small library catalogue from
files, routes rows onto the constructors of a sum, keeps the rows that
do not fit, and stages rows for repair. The full rules are in
[file scans](../../spec/interop.md#3-file-scans) and
[loading](../../spec/interop.md#4-loading).

Save this schema as `library.adt`:

```adt library.adt
module library

type Author = { name: Text, born: I64 }
relation authors : Author key name

type Book { isbn: Text, title: Text, author: Key authors }
  = Print { pages: I64 }
  | Ebook { bytes: I64 }
relation books : Book key isbn

type BookRow = { isbn: Text, title: Text, author: Key authors, format: Tag Book, size: I64 }

relation load_rejects : RejectedRow key (load, line)
```

`BookRow` is the shape of one line of the books file. It is a type
with no relation: a file is read at a type you declare, never at one
guessed from the file.

```console
$ adtb --create --schema library.adt library.db
adtb> \q
```

## Read a file

Save the authors as `authors.csv`:

```csv authors.csv
name,born
Ursula Le Guin,1929
Italo Calvino,1923
```

`read_csv(path, T)` is the set of the file's rows read at type `T`.
Each field is read from the column of the same name; columns the type
does not name are ignored. A path is read relative to the directory
the shell runs in.

```console
$ adtb library.db
adtb> from a in read_csv("authors.csv", Author) select a order by a.name
Author { name: "Italo Calvino", born: 1923 }
Author { name: "Ursula Le Guin", born: 1929 }
adtb> from a in read_csv("authors.csv", Author) where a.born < 1925 select a.name
"Italo Calvino"
adtb> \q
```

A file is not a relation of the database: no key, constraint or
index ranges over it, and it can be read only in ad hoc queries and in
the source of an insert.

## Load it

A load is an `insert` whose rows come from a query over the file. It
is one statement, checked like any other, so either every row lands
or none does:

```console
$ adtb library.db
adtb> insert authors from (from a in read_csv("authors.csv", Author) select a)
adtb> from a in authors select a.name order by a.name
"Italo Calvino"
"Ursula Le Guin"
adtb> \q
```

## Route rows onto constructors

A file has no sums. The books file says which constructor each row is
in a `format` column. Save it as `books.csv`:

```csv books.csv
isbn,title,author,format,size
0441478123,The Left Hand of Darkness,Ursula Le Guin,Print,304
0156453800,Invisible Cities,Italo Calvino,Ebook,1200000
0151000000,Mr Palomar,Italo Calvino,Scroll,120
0099000000,Numbers in the Dark,Italo Calvino,Print,many
```

`format` is read into a field of type `Tag Book`, the type whose
values are the names of `Book`'s constructors, and `author` into a
`Key authors`, read from the author's name. A `match` on the tag then
builds the right constructor. Because the match names every
constructor, a constructor added to `Book` later makes this load stop
checking until you route it.

Two rows of this file do not convert: `Scroll` is not a constructor,
and `many` is not a number. Without anywhere to put them, the whole
load fails, naming the file, the record and the column:

```console
$ adtb library.db
adtb> insert books from (from r in read_csv("books.csv", BookRow) select match r.format | Print -> Print { isbn: r.isbn, title: r.title, author: r.author, pages: r.size } | Ebook -> Ebook { isbn: r.isbn, title: r.title, author: r.author, bytes: r.size })
books.csv:3: column `format`: `Scroll` is not a constructor
adtb> \q
```

The number after the file name is the record's position in the file,
counting the first record after the header as 1.

## Keep the rows that do not fit

`rejects into r` sends each record that fails to convert to the
relation `r` and loads every other row. `r` has the prelude type
`RejectedRow`, with fields `load`, `line`, `source`, `raw` and
`reason`, keyed by `(load, line)`; `load` is a new id for each loading
statement, so a rerun never collides with an earlier one.

```console
$ adtb library.db
adtb> insert books from (from r in read_csv("books.csv", BookRow) select match r.format | Print -> Print { isbn: r.isbn, title: r.title, author: r.author, pages: r.size } | Ebook -> Ebook { isbn: r.isbn, title: r.title, author: r.author, bytes: r.size }) rejects into load_rejects
adtb> from j in load_rejects select { line: j.line, raw: j.raw, reason: j.reason } order by j.line
{ line: 3, raw: "0151000000,Mr Palomar,Italo Calvino,Scroll,120", reason: "column `format`: `Scroll` is not a constructor" }
{ line: 4, raw: "0099000000,Numbers in the Dark,Italo Calvino,Print,many", reason: "column `size`: at byte 0: expected a digit" }
adtb> from b in books select { title: b.title, by: b.author.name, kind: tag(b) } order by b.title
{ by: "Italo Calvino", kind: Ebook, title: "Invisible Cities" }
{ by: "Ursula Le Guin", kind: Print, title: "The Left Hand of Darkness" }
adtb> \q
```

`reject "why"` in the source query sends a row to the rejects relation
on your own terms, with your message as its reason. It may appear only
in the source of an insert.

A constraint violation is not a reject. A row that converts but breaks
a constraint, such as a book whose author is not in `authors`, fails
the whole statement and keeps no rejects. Filter such rows out in the
source query, or load what they refer to first.

## Read NDJSON

`read_ndjson` reads one JSON object per line. A row of a sum is an
object with one member named by its constructor, holding the fields.
Numbers of type `I64` and other wide scalars are written as strings
holding their textual form, so they never lose precision. Save this as
`more.ndjson`:

```json more.ndjson
{"Print": {"isbn": "0060000000", "title": "If on a winter's night a traveller", "author": "Italo Calvino", "pages": "260"}}
{"Ebook": {"isbn": "0061054887", "title": "The Dispossessed", "author": "Ursula Le Guin", "bytes": "900000"}}
```

Read at `Book` itself, the file loads straight onto its constructors:

```console
$ adtb library.db
adtb> insert books from (from b in read_ndjson("more.ndjson", Book) select b)
adtb> from b in books select { title: b.title, kind: tag(b) } order by b.title
{ kind: Print, title: "If on a winter's night a traveller" }
{ kind: Ebook, title: "Invisible Cities" }
{ kind: Ebook, title: "The Dispossessed" }
{ kind: Print, title: "The Left Hand of Darkness" }
adtb> \q
```

A missing member of an `Option` field reads as `None`; any other
missing member is a conversion failure.

## Read several files as one

A path may hold a glob, and then names every file it matches, read as
one relation. Save two monthly loan files:

```csv loans-2026-09.csv
isbn,days
0441478123,14
0156453800,7
```

```csv loans-2026-10.csv
isbn,days
0441478123,21
```

```adt loans.adt
module library
add type LoanRow = { isbn: Text, days: I64 }
```

```console
$ adtb --schema library.adt --schema loans.adt library.db
adtb> from l in read_csv("./loans-2026-*.csv", LoanRow), b in books where l.isbn == b.isbn group l by b.title into g select { title: g.key, days: sum(x.days for x in g) }
{ days: 7, title: "Invisible Cities" }
{ days: 35, title: "The Left Hand of Darkness" }
adtb> \q
```

Write the glob with a directory part, as `./` here: in this engine a
glob with no directory part matches no file, and the scan is empty.
`LoanRow` was added by a migration, because the database already
exists; [Evolving a schema](evolving-a-schema.md) covers migrations.

## Stage rows for repair

When a file needs fixing before it fits, load it into a temp relation
first, repair it with ordinary updates, and then move it. A temp
relation is declared in ad hoc text with `temp relation`, is not part
of the schema, and lives until the transaction that declares it ends.
Inside an interactive transaction, opened with `\begin`, it lasts
across several texts.

The rejected rows above had a misspelt format and a size in words.
Save the corrected rows, still with the bad format, as `fixes.csv`:

```csv fixes.csv
isbn,title,author,format,size
0151000000,Mr Palomar,Italo Calvino,paper,120
0099000000,Numbers in the Dark,Italo Calvino,paper,192
```

```adt staging.adt
module library
add type BookText = { isbn: Text, title: Text, author: Key authors, format: Text, size: I64 }
```

```console
$ adtb --schema library.adt --schema loans.adt --schema staging.adt library.db
adtb> \begin
adtb> temp relation staged : BookText key isbn
adtb> insert staged from (from r in read_csv("fixes.csv", BookText) select r)
adtb> update s in staged where s.format == "paper" set BookText { s with format: "Print" }
adtb> insert books from (from s in staged select match (Tag Book).parse(s.format) | Some { value: Print } -> Print { isbn: s.isbn, title: s.title, author: s.author, pages: s.size } | Some { value: Ebook } -> Ebook { isbn: s.isbn, title: s.title, author: s.author, bytes: s.size } | None -> reject "unknown format") rejects into load_rejects
adtb> \commit
adtb> from b in books.Print select b.title order by b.title
"If on a winter's night a traveller"
"Mr Palomar"
"Numbers in the Dark"
"The Left Hand of Darkness"
adtb> \q
```

`(Tag Book).parse(t)` reads a constructor name from text, returning
`None` for a name that is not one. The temp relation is gone once the
transaction commits; only the books remain.
