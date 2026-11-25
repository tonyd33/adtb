# Documentation

The site is built by `mdbook` from `SUMMARY.md`; a page under `manual/`, `spec/` or `design/` is
listed there.

- A file under `design/` states what one subsystem promises another: its operations, their
  preconditions and postconditions, and the invariants a caller may rely on. It names no struct
  field, private item, parameter list or signature; those change with the code while the contract
  stays.
- A sentence claiming every, each or only the forms of a set names what keeps it true: an
  exhaustive match, a closed type or a test that searches. With none, list the forms known and
  make no such claim.
- State a fact once, in the file that owns it, and link to it elsewhere.
