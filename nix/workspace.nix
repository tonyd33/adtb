pkgs:

let
  inherit (pkgs) lib rustPlatform;

  toolchain = (lib.importTOML ../rust-toolchain.toml).toolchain.channel;

  source =
    files:
    lib.fileset.toSource {
      root = ../.;
      fileset = lib.fileset.unions (
        [
          ../Cargo.toml
          ../Cargo.lock
          ../crates
        ]
        ++ files
      );
    };

  common = {
    version = (lib.importTOML ../Cargo.toml).workspace.package.version;
    cargoLock.lockFile = ../Cargo.lock;
  };

  binaries = {
    adtb-shell = "adtb";
    adtb-codegen = "adtb-codegen";
    adtb-probe = "adtb-probe";
    adtb-conformance = "adtb-conformance";
  };

  workspace = rustPlatform.buildRustPackage (
    common
    // {
      pname = "adtb";
      src = source [ ];
      cargoBuildFlags = lib.concatMap (crate: [
        "--package"
        crate
      ]) (lib.attrNames binaries);
      doCheck = false;
      meta = {
        description = "A database engine with native sum types";
        license = lib.licenses.mit;
        mainProgram = "adtb";
      };
    }
  );

  binary =
    crate: program:
    pkgs.runCommand crate
      {
        meta = workspace.meta // {
          mainProgram = program;
        };
      }
      ''
        mkdir -p $out/bin
        ln -s ${workspace}/bin/${program} $out/bin/${program}
      '';
in
assert lib.assertMsg (
  pkgs.rustc.version == toolchain
) "rust-toolchain.toml pins Rust ${toolchain} but nixpkgs supplies ${pkgs.rustc.version}";
{
  inherit workspace;

  binaries = lib.mapAttrs binary binaries;

  tests = rustPlatform.buildRustPackage (
    common
    // {
      pname = "adtb-tests";
      src = source [
        ../.config/nextest.toml
        ../conformance
        ../docs/spec
        ../docs/manual
        ../apps/campaign/schema
        ../apps/campaign/bundles
      ];
      useNextest = true;
      checkType = "test";
      cargoTestFlags = [ "--workspace" ];
      dontCargoBuild = true;
      installPhase = "touch $out";
    }
  );
}
