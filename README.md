# adtb

A database engine with native sum types at the value and relation levels.
Written in Rust, with its own storage engine and a comprehension-based
query language.

## Development

Use Linux with [Nix](https://nixos.org/download/) and
[devenv](https://devenv.sh/getting-started/). From a clone, run:

```sh
devenv shell
jj git init --colocate
cargo build --workspace
work/tools/suite
```

If you cloned with `jj git clone`, skip `jj git init --colocate`.
Run `check` in the dev shell for the full checks.

See [development setup](docs/development.md) for prerequisites, optional
direnv and editor integration, disk-space settings, and moving work
between machines. Project tools are listed in [AGENTS.md](AGENTS.md#tools).

## Documentation

- [User manual](docs/manual/README.md): getting started and language usage.
- [Specification](docs/spec/README.md): observable language and database behaviour.
- [Subsystem contracts](docs/design/README.md): interfaces between engine components.
- [Implementation plan](docs/plan.md).
- [Tickets](work/issues/README.md): tracked work and ticket conventions.
- [Knowledge base](work/kb/README.md): research facts and sources.
- [Design space](work/research/): questions, prior art and adopted rulings;
  run `quandary status` there to see the frontier.

## Coding agents

Read [AGENTS.md](AGENTS.md) before working in the repository. It contains
the shared instructions, commands and project conventions. Harnesses that
do not discover it automatically can be pointed at it explicitly.

Reusable workflows live in [.agents/skills/](.agents/skills/); read their
`SKILL.md` files directly if your harness has no skill loader. Claude Code
entry points import or reference these shared files.
