# Development setup

## A fresh machine

The full development workflow targets Linux. The real-file runtime uses
Linux direct I/O; its filesystem must support `O_DIRECT`. On macOS or
Windows, use a Linux VM for the full workflow.

Install [Nix](https://nixos.org/download/) and
[devenv](https://devenv.sh/getting-started/). Nix must have the
`nix-command` and `flakes` experimental features enabled. You need network
access for the initial dependency downloads and credentials for the
repository if cloning over SSH.

From an existing Git clone:

```sh
devenv shell
jj git init --colocate
jj status
cargo build --workspace
cargo xtask test
```

Alternatively, install jj on the host and use `jj git clone REPOSITORY_URL
adtb`, then enter that directory and run `devenv shell`. A jj clone already
has its version-control state initialized.

Configure your jj author identity on a new machine before committing:

```sh
jj config set --user user.name "Your Name"
jj config set --user user.email "you@example.com"
```

The dev shell supplies Rust, jj, tql, tree-sitter, mdbook, lychee,
cargo-nextest, cargo-deny and the shell utilities. You do not need a local
checkout of editorenv. Nix dependencies are pinned in
`devenv.lock`; Cargo dependencies are pinned in the workspace's `Cargo.lock`. Keep
these lockfiles when moving machines.
The Rust toolchain is recorded in `rust-toolchain.toml`.

Direnv is optional. If installed and hooked into your shell, `direnv allow`
activates the environment automatically. Otherwise, use `devenv shell`, or
run a command directly with `devenv shell cargo build --workspace`.

## Checks and disk space

Run commands from the repository root:

```sh
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo xtask test --profile quick
cargo xtask check
```

`cargo xtask check` runs the full workspace checks, dependency
policy and source-size limits, in a systemd user scope whose memory the kernel
limits. `cargo xtask test` runs the tests in a scope of its own and saves
output to `target/test.txt`. To run only the Loom models or Miri:

```sh
RUSTFLAGS='--cfg adtb_loom' CARGO_TARGET_DIR=target/loom cargo nextest run -p adtb-sys --lib pages::loom_tests::
PATH=$ADTB_NIGHTLY_BIN:$PATH cargo miri test -p adtb-sys --lib
```

A test that needs io_uring fails on a host that refuses it; set `ADTB_URING=absent` there to skip those tests with a note on stderr.

Builds stay in each workspace's `target/`; `CARGO_TARGET_DIR` can select
another location.

The dev and test profiles carry no debug info, to keep `target/` small. A panic
message still names its file and line; a backtrace names only functions, and loses inlined frames. For
file and line in a backtrace, rebuild with line tables, which recompiles
everything the test needs:

```sh
CARGO_PROFILE_TEST_DEBUG=line-tables-only RUST_BACKTRACE=1 cargo nextest run -p CRATE FILTER
```

## Build with Nix

`flake.nix` builds the binaries outside the dev shell, for x86_64 and aarch64 Linux. It
takes nixpkgs from `devenv.lock` and checks that its Rust matches
`rust-toolchain.toml`, so updating devenv updates both.

```sh
nix build .#                      # every binary, in result/bin
nix run .# -- --help              # the adtb shell
nix run .#adtb-conformance -- --help
nix flake check                   # the packages and the test suite
```

The packages are `adtb-shell` (the `adtb` binary), `adtb-codegen`,
`adtb-conformance` and `default`, which holds all three. The packaged
`adtb-conformance` runs only the conformance files: the engine scripts need the
`scripts` feature, which `cargo conformance` enables. Nix
reads the Git index, so a file new in the working copy is invisible to the
flake until Git tracks it: `git add --intent-to-add FILE`.

## Read the documentation locally

In the development shell, run `mdbook build` to build the manual, specification
and design documentation into `target/docs-site/html/`. Run
`devenv processes up docs` to serve it with live reload at
<http://127.0.0.1:8081/> (or the port devenv assigns if it is occupied).
The manual's first page is `/manual/getting-started.html`.

For a standalone server, run `mdbook serve --port 8081`.
The site binds only to loopback. Search, themes and highlighting work without
an internet connection. No Node installation is required.

The checked-in `book.toml` reads `docs/` directly. `docs/SUMMARY.md` lists every
page: add a new page under `docs/manual/`, `docs/spec/` or `docs/design/` there. The
suite fails on a page left out and on a listed path that does not exist. `mdbook
serve` rebuilds on an edit and watches the highlighting asset. `mdbook build` and
`mdbook serve --hostname 127.0.0.1` also work directly from the repository root.
After `mdbook build`, `lychee --offline --include-fragments --root-dir target/docs-site/html
target/docs-site/html` checks every internal link and fragment. The suite compares the highlighter keywords with the
lexer table. `cargo xtask check` runs both. Hosting is a separate task.

## Optional editor integration

Use any editor. Personal configuration belongs in the ignored
`devenv.local.nix` and `devenv.local.yaml` files.

To use editorenv, add this to `devenv.local.yaml`:

```yaml
inputs:
  editorenv:
    url: github:tonyd33/editorenv
    inputs:
      nixpkgs:
        follows: nixpkgs
imports:
  - editorenv/integrations/devenv
```

Merge these settings into `devenv.local.nix`:

```nix
{
  editors.neovim.enable = true;
  languages.rust.lsp.enable = true;
}
```

Re-enter the shell after changing these files. Adding a local input may
update `devenv.lock`; keep personal editor-input changes out of shared
commits.

## Moving work between machines

A clone contains published Git history and refs. It does not contain
uncommitted changes, unpublished jj changes, jj workspace registrations,
local configuration, or an agent harness's private memory. Describe and
publish the changes you need before switching machines.

Read [AGENTS.md](../AGENTS.md) with any coding agent. Keep resumable task
state in tracked files, including paths, commands and unresolved failures.
Verify essential paths with `jj file list PATH...`; an ignored file is not
part of a clone merely because a local tool can read it.

Global ignore rules are personal. Explicitly track a project file they
match with `jj file track --include-ignored PATH` (or `git add -f PATH`
when using Git). Tracked files remain tracked without `.gitignore`
exceptions.
