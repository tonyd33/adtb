{
  pkgs,
  lib,
  config,
  inputs,
  ...
}:

let
  quandary = inputs.quandary.packages.${pkgs.stdenv.system}.default;
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
    cargo build --workspace --all-targets
    cargo clippy --workspace --all-targets -- -D warnings
    cargo fmt --all --check
    cargo nextest run --workspace --no-tests=pass
    cargo deny check
    size-check
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
