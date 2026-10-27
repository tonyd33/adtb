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
in
{
  # https://devenv.sh/packages/
  packages = [
    pkgs.git
    pkgs.cargo-deny
    pkgs.cargo-nextest
    quandary
  ];

  scripts.check.exec = ''
    set -e
    export CARGO_TARGET_DIR="''${CARGO_TARGET_DIR:-$DEVENV_ROOT/target}"
    cargo build --workspace --all-targets
    cargo clippy --workspace --all-targets -- -D warnings
    cargo fmt --all --check
    cargo nextest run --workspace --no-tests=pass
    app="$DEVENV_ROOT/apps/campaign/Cargo.toml"
    diff <(sed -n '/^\[workspace.lints.rust\]/,$p' "$DEVENV_ROOT/Cargo.toml") \
      <(sed -n '/^\[workspace.lints.rust\]/,$p' "$app")
    cargo build --manifest-path "$app" --all-targets
    cargo clippy --manifest-path "$app" --all-targets -- -D warnings
    cargo fmt --manifest-path "$app" --all --check
    cargo nextest run --manifest-path "$app" --config-file "$DEVENV_ROOT/.config/nextest.toml"
    cargo deny check
    size-check
  '';

  scripts.tix.exec = ''
    ${lib.getExe tix} -C "$DEVENV_ROOT/work/issues" "$@"
  '';

  scripts.size-check.exec = ''
    "$DEVENV_ROOT/work/tools/size-check.sh"
  '';

  scripts.quality-metrics.exec = ''
    "$DEVENV_ROOT/work/tools/quality-metrics.sh" "$@"
  '';

  enterTest = "check";

  # https://devenv.sh/languages/
  languages.rust = {
    enable = true;
    lsp.enable = true;
  };

  # https://devenv.sh/processes/
  # processes.dev.exec = "${lib.getExe pkgs.watchexec} -n -- ls -la";
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

  editors.neovim.enable = true;

  # See full reference at https://devenv.sh/reference/options/
}
