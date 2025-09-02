{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    nixpkgs-ocitools.url = "github:msanft/nixpkgs/msanft/oci/refactor";

    systems.url = "github:nix-systems/default";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    systems,
    crane,
    ...
  } @ inputs: let
    inherit (nixpkgs) lib;

    forEachSystem = fun:
      lib.genAttrs (import systems) (
        system: fun (import nixpkgs {inherit system;})
      );
  in {
    packages = forEachSystem (
      pkgs: let
        craneLib = crane.mkLib pkgs;
        commonArgs = {
          src = craneLib.cleanCargoSource ./.;
          strictDeps = true;
          buildInputs =
            [pkgs.nix pkgs.nix-eval-jobs]
            ++ lib.optionals pkgs.stdenv.isDarwin [pkgs.libiconv];
        };
      in {
        default = craneLib.buildPackage (
          commonArgs
          // {
            cargoArtifacts = craneLib.buildDepsOnly commonArgs;
            meta.mainProgram = "steiger";

            NIX_BINARY = lib.getExe pkgs.nix;
            NIX_EVAL_JOBS_BINARY = lib.getExe pkgs.nix-eval-jobs;
          }
        );
      }
    );

    checks = forEachSystem (pkgs: {
      inherit (self.packages.${pkgs.system}) default;
    });

    devShells = forEachSystem (
      pkgs: let
        craneLib = crane.mkLib pkgs;
      in {
        default = craneLib.devShell {
          packages = [
            pkgs.nix-eval-jobs
          ];
        };
      }
    );

    overlays.ociTools = final: prev: let
      pkgs = import inputs.nixpkgs-ocitools {inherit (final) system;};
    in {
      inherit (pkgs) ociTools;
    };
  };
}
