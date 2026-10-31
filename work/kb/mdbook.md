# mdBook

kind: system
home: https://rust-lang.github.io/mdBook/

## Documentation sites

- Markdown links ending .md are converted to .html; README.md links map to index.html, and heading fragments derive from heading text. [V 2026-10-27] https://rust-lang.github.io/mdBook/format/markdown.html#links
- The HTML renderer provides search and themes. Additional JavaScript is supported; highlighting uses Highlight.js, whose bundled language set does not include adtb. Custom highlight.js can replace the bundled file; replacing theme files makes their future upstream updates the project’s responsibility. [V 2026-10-27] https://rust-lang.github.io/mdBook/format/configuration/general.html https://rust-lang.github.io/mdBook/format/theme/syntax-highlighting.html https://rust-lang.github.io/mdBook/format/theme/index.html
- Local adtb prototype using mdBook 0.5.4 built all 57 manual/spec/design Markdown pages unchanged via symlinks. Disable the default index preprocessor with `[build] use-default-preprocessors = false`, then enable `[preprocessor.links]`; this preserves README.html output and matches emitted relative links. The default index preprocessor renamed README outputs while emitted links still named README.html in this corpus. An explicit site-root landing page avoids copying the first nested page to root with invalid relative paths. [V 2026-10-27] Local reproducible evidence: target/docs-prototype/mdbook/book.toml, mdbook-out-adapted-links.txt in the docs-site-research workspace; option docs https://rust-lang.github.io/mdBook/format/configuration/general.html#build-options
- The adapted corpus produced 2,985 internal anchor links, including 2,336 fragments, with zero missing targets or fragments. Chromium verified a numbered `83-fields` anchor, adtb keyword/type spans through an additional-js Highlight.js registration, and search results spanning manual, spec and design without failed requests or browser errors. This is a prototype observation, not proof of a complete syntax grammar. [V 2026-10-27] Local evidence: target/docs-prototype/mdbook-browser-check.log and mdbook-out-adapted-links.txt in docs-site-research workspace.
- With symlinked source sections, configure `[build] extra-watch-dirs` to the real docs directory. `mdbook serve` rebuilt on a real source content change with this explicit watch path; the source was restored byte-for-byte. The poll watcher is the default in 0.5.4. [V 2026-10-27] https://rust-lang.github.io/mdBook/format/configuration/general.html#build-options and local target/docs-prototype/mdbook-serve-watch.log; `mdbook serve --help`.

## Watching a generated source view

- mdBook0.5.4's poll watcher scans once per second, follows symlinks, and filters canonical paths through the nearest ancestor `.gitignore`. An ignored generated source root can be pruned before its children are visited. A generated empty `.gitignore` in the book root isolates the watcher from the repository's `target/` exclusion. [V 2026-10-28] https://github.com/rust-lang/mdBook/blob/v0.5.4/src/cmd/watch/poller.rs and https://github.com/rust-lang/mdBook/blob/v0.5.4/src/cmd/watch.rs; local docs-nav regression `target/docs-nav-poll-override.log`.
- In the local symlink-source fixture, native watching rebuilt continuously at idle (9 to12 builds over1.25s), whereas polling with the book-local ignore boundary remained idle and propagated navigation/assets to HTTP. This is a measured fixture outcome; the native event-loop cause is not established. [V 2026-10-28] docs-nav `target/docs-nav-complete.log`, `target/docs-nav-poll-override.log`; native watcher implementation https://github.com/rust-lang/mdBook/blob/v0.5.4/src/cmd/watch/native.rs.

## Native navigation extension

- A configured preprocessor receives `[context, book]` JSON and returns a modified Book on stdout; it can add chapters in memory before rendering. Its `supports html` invocation exits successfully, and `before = ["links"]` orders navigation expansion before built-in link processing. [V 2026-10-28] https://rust-lang.github.io/mdBook/for_developers/preprocessors.html and https://rust-lang.github.io/mdBook/format/configuration/preprocessors.html; installed0.5.4 prototype target/native-mdbook-prototype/build.log and input.json.
- The0.5.4 poll watcher watches the source tree, book.toml, theme and additional JS/CSS paths directly. A book rooted at the repository with `src = "docs"` can therefore observe authored Markdown and the grammar asset without a generated source view or a second watcher. [V 2026-10-28] https://raw.githubusercontent.com/rust-lang/mdBook/v0.5.4/src/cmd/watch/poller.rs lines113-137.
- Installed0.5.4 resolves a relative preprocessor command from the book root, including when `mdbook build /absolute/book/path` is invoked from another directory. [V 2026-10-28] local target/native-mdbook-prototype/relative-hook.log/status; fixture command `python3 -B navigation.py`.

## Default preprocessors

- With the default preprocessors in mdBook 0.5.4 and a full SUMMARY.md, the index preprocessor writes `spec/README.md` as `spec/index.html`, but a Markdown link to `README.md` still emits `README.html`: 49 links in this repository's docs fail a link check. `use-default-preprocessors = false` with `[preprocessor.links]` keeps `README.html` and all links resolve. [V 2026-10-30] local build and lychee run.
- mdBook fails the build on a SUMMARY.md path that does not exist when `create-missing = false`, and says nothing about a Markdown file left out of SUMMARY.md. [V 2026-10-30] local build, exit status read.
