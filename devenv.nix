{
  pkgs,
  lib,
  config,
  inputs,
  ...
}:

let
  quandary = inputs.quandary.packages.${pkgs.stdenv.system}.default;
  tix = inputs.tix.packages.${pkgs.stdenv.system}.default;
  nightly = (inputs.rust-overlay.lib.mkRustBin { } pkgs).selectLatestNightlyWith (
    toolchain:
    toolchain.minimal.override {
      extensions = [
        "miri"
        "rust-src"
      ];
    }
  );
in
{
  # https://devenv.sh/packages/
  packages = [
    pkgs.bash
    pkgs.coreutils
    pkgs.diffutils
    pkgs.findutils
    pkgs.git
    pkgs.gawk
    pkgs.gnugrep
    pkgs.gnused
    pkgs.jujutsu
    pkgs.jq
    pkgs.lychee
    pkgs.mdbook
    pkgs.ripgrep
    pkgs.tree-sitter
    pkgs.util-linux
    pkgs.cargo-deny
    pkgs.cargo-nextest
    quandary
  ];

  scripts.check.exec = ''
    set -e
    source "$DEVENV_ROOT/work/tools/common.bash"
    machine_lock "$DEVENV_ROOT"
    export CARGO_TARGET_DIR="''${CARGO_TARGET_DIR:-$DEVENV_ROOT/target}"
    cargo clippy --workspace --all-targets -- -D warnings
    cargo fmt --all --check
    suite --no-tests=pass
    loom
    miri
    app="$DEVENV_ROOT/apps/campaign/Cargo.toml"
    diff <(sed -n '/^\[workspace.lints.rust\]/,$p' "$DEVENV_ROOT/Cargo.toml") \
      <(sed -n '/^\[workspace.lints.rust\]/,$p' "$app")
    cargo clippy --manifest-path "$app" --all-targets -- -D warnings
    cargo fmt --manifest-path "$app" --all --check
    cargo nextest run --manifest-path "$app" --config-file "$DEVENV_ROOT/.config/nextest.toml"
    cargo deny check
    cargo deny --manifest-path "$app" check
    size-check
    docs-check
    tree-sitter-check
  '';

  scripts.docs-build.exec = ''
    "$DEVENV_ROOT/work/tools/docs-site" build
  '';

  scripts.docs-check.exec = ''
    "$DEVENV_ROOT/work/tools/docs-site" check
  '';

  scripts.tix.exec = ''
    ${lib.getExe tix} -C "$DEVENV_ROOT/work/issues" "$@"
  '';

  scripts.miri.exec = ''
    PATH="${nightly}/bin:$PATH" "$DEVENV_ROOT/work/tools/miri" "$@"
  '';

  scripts.loom.exec = ''
    "$DEVENV_ROOT/work/tools/loom" "$@"
  '';

  scripts.size-check.exec = ''
    "$DEVENV_ROOT/work/tools/size-check.sh"
  '';

  scripts.suite.exec = ''
    "$DEVENV_ROOT/work/tools/suite" "$@"
  '';

  scripts.conf.exec = ''
    "$DEVENV_ROOT/work/tools/conf" "$@"
  '';

  scripts.codemap.exec = ''
    "$DEVENV_ROOT/work/tools/codemap" "$@"
  '';

  scripts.tree-sitter-check.exec = ''
    "$DEVENV_ROOT/work/tools/tree-sitter-check" "$@"
  '';

  scripts.house-check.exec = ''
    "$DEVENV_ROOT/work/tools/house-check" "$@"
  '';

  scripts.workspace-gc.exec = ''
    "$DEVENV_ROOT/work/tools/workspace-gc" "$@"
  '';

  scripts.slot.exec = ''
    "$DEVENV_ROOT/work/tools/slot" "$@"
  '';

  scripts.slot-test.exec = ''
    "$DEVENV_ROOT/work/tools/slot-test" "$@"
  '';

  scripts.adtb-probe.exec = ''
    "$DEVENV_ROOT/work/tools/adtb-probe" "$@"
  '';

  scripts.quality-metrics.exec = ''
    "$DEVENV_ROOT/work/tools/quality-metrics.sh" "$@"
  '';

  scripts.code-commits.exec = ''
    "$DEVENV_ROOT/work/tools/code-commits" "$@"
  '';

  scripts.land.exec = ''
    "$DEVENV_ROOT/work/tools/land" "$@"
  '';

  enterTest = "check";

  # https://devenv.sh/languages/
  languages.rust = {
    enable = true;
  };

  # https://devenv.sh/processes/
  # processes.dev.exec = "${lib.getExe pkgs.watchexec} -n -- ls -la";
  processes.docs =
    let
      docsPort = config.processes.docs.ports.http.value;
    in
    {
      ports.http.allocate = 8081;
      exec = ''
        "$DEVENV_ROOT/work/tools/docs-site" serve \
          --port ${toString docsPort} \
          --hostname 0.0.0.0
      '';
      proxy.hostname = "adtb-docs.localhost";
      ready.http.get = {
        port = docsPort;
        path = "/";
      };
    };

  processes.quandary =
    let
      quandaryPort = config.processes.quandary.ports.http.value;
    in
    {
      ports.http.allocate = 8080;
      exec = ''
        ${lib.getExe quandary} -C work/research serve \
          --port ${toString quandaryPort} \
          --bind 0.0.0.0
      '';
      proxy.hostname = "adtb-qdry.localhost";
      ready = {
        http.get = {
          port = quandaryPort;
          path = "/";
        };
      };
    };

  # https://devenv.sh/services/
  # services.postgres.enable = true;

  # https://devenv.sh/tasks/
  # tasks = {
  #   "myproj:setup".exec = "mytool build";
  #   "devenv:enterShell".after = [ "myproj:setup" ];
  # };

  # https://devenv.sh/git-hooks/
  git-hooks.hooks = {
    shellcheck.enable = true;
    nixfmt.enable = true;
  };

  # See full reference at https://devenv.sh/reference/options/
}
