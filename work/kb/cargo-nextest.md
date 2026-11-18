# cargo-nextest

kind: system
home: https://nexte.st

## Machine-readable output

- `--message-format libtest-json` and `libtest-json-plus` need `NEXTEST_EXPERIMENTAL_LIBTEST_JSON=1`.
  They write test events to standard output while the human reporter stays on standard error;
  `--cargo-message-format json` adds cargo's JSON messages to the same standard output.
  [V 2026-11-18] cargo-nextest 0.9.146, a scratch crate with passing, failing, aborting,
  hanging and ignored tests
- The libtest-json stream loses a timeout: the timed out test gets no `failed` event, a second
  `suite` `started` event follows, and its `suite` end event counts one failure with the other
  tests as `filtered_out`. An abort shows as a plain `failed` event with no signal. The human
  summary on standard error names each as `TIMEOUT` and `SIGABRT`. [V 2026-11-18]
  cargo-nextest 0.9.146, same crate
- The human summary starts at a line `Summary [TIME]` after a rule and lists every failed,
  killed and timed out test after it. [V 2026-11-18] cargo-nextest 0.9.146, same crate
