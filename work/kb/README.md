# Knowledge base

Facts about databases, formats, languages and papers gathered while
researching the design space. Read here before fetching anything; write
here after fetching.

## Layout

One file per subject, flat: `postgres.md`, `duckdb.md`, `tigerbeetle.md`,
`prism-bidel.md`. A paper or paper family gets its own file. Name files
in lowercase kebab-case after the subject, not the topic it was fetched
for.

Each file:

```
# <Subject>

kind: system | format | language | paper
home: <canonical URL>

## <Topic>

- <fact>. [V 2026-10-25] <source URL or citation>
```

Topics are free headings (storage, indexes, concurrency, types, query
language, constraints, evolution, ...).

## Marks

- `[V date]` checked in the primary source on that date.
- `[S date]` seen only in a search snippet or secondary source.
- `[U]` unverified, from memory or inference.

A fact without a source is `[U]`. Upgrade a mark only after reading the
primary source.

## Rules

- Before a WebSearch or WebFetch, `grep -ri <term> work/kb/`. Fetch only
  what is missing or marked `[U]`/`[S]` and needed at `[V]`.
- Append facts; do not rewrite others' bullets except to upgrade a mark
  or correct a fact, noting the correction's source.
- Several agents may write at once: append with `cat >> file <<'EOF'`,
  never rewrite a whole file another writer may hold.
- WebFetch summaries of PDFs invent facts. Mark a paper fact `[V]` only
  after reading its text: `nix shell nixpkgs#poppler-utils -c pdftotext
  paper.pdf -`.
- Quandary `notes/prior/` notes stay short summaries for linking; detail
  lives here.

## Queue

Subjects named for research with nothing recorded yet: none.
