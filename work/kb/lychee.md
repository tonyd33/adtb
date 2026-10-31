# lychee

kind: system
home: https://lychee.cli.rs/

## Checking a built site offline

- nixpkgs (devenv-nixpkgs rolling, rev 760b8522) packages lychee 0.24.2. [V 2026-10-30] local `nix build`, `lychee --version`.
- `lychee --offline --include-fragments --root-dir SITE SITE` over the built mdBook 0.5.4 site of 63 pages checks 6473 links in about 1 s, excludes the 13 remote ones and reports 0 errors. It exits 2 on any failure. [V 2026-10-30] local run on this repository's `target/docs-site/html`.
- That command fails on a planted missing page, a planted missing fragment on another page, on the same page and on a README page, and a planted missing page in another directory. [V 2026-10-30] local run, planted in a scratch copy of the site.
- `--root-dir` must be absolute; without it, mdBook's `404.html` fails on the root-relative link `/`. [V 2026-10-30] local run.
