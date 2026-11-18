# Conformance suite

One directory per spec part; the format is `docs/spec/conformance.md`.
`cargo conformance --file-name PATH`, from the workspace root, runs files or directories on the reference and the
engine and prints each difference with its file and line.
