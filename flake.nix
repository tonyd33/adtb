{
  description = "adtb - a database engine with native sum types";

  outputs =
    _:
    let
      lock = builtins.fromJSON (builtins.readFile ./devenv.lock);
      fetchLocked = node: builtins.fetchTree lock.nodes.${node}.locked;
      nixpkgsNode = lock.nodes.root.inputs.nixpkgs;
      nixpkgs = (import (fetchLocked nixpkgsNode + "/flake.nix")).outputs {
        self = fetchLocked nixpkgsNode;
        nixpkgs-src = fetchLocked lock.nodes.${nixpkgsNode}.inputs.nixpkgs-src;
      };
      forAllSystems =
        f:
        builtins.listToAttrs (
          map
            (system: {
              name = system;
              value = f (import ./nix/workspace.nix nixpkgs.legacyPackages.${system});
            })
            [
              "x86_64-linux"
              "aarch64-linux"
            ]
        );
    in
    {
      packages = forAllSystems (adtb: adtb.binaries // { default = adtb.workspace; });

      checks = forAllSystems (adtb: adtb.binaries // { inherit (adtb) workspace tests; });
    };
}
