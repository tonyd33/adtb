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
    tix
  ];

  env.ADTB_NIGHTLY_BIN = "${nightly}/bin";

  enterTest = "cargo xtask check";

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
        mdbook serve "$DEVENV_ROOT" \
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

  editors.neovim.enable = true;

  # See full reference at https://devenv.sh/reference/options/
}
