{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    nixpkgs-ocitools.url = "github:msanft/nixpkgs/msanft/oci/refactor";

    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    crane,
    ...
  } @ inputs: let
    inherit (nixpkgs) lib;

    forEachSystem = fun:
      lib.genAttrs (lib.systems.flakeExposed) (
        system: fun (import nixpkgs {inherit system;})
      );
  in {
    lib = {
      defaultSystems = ["aarch64-darwin" "x86_64-darwin" "x86_64-linux" "aarch64-linux"];

      eachCrossSystem = systems: packages:
        nixpkgs.lib.genAttrs systems (localSystem:
          nixpkgs.lib.genAttrs systems (crossSystem:
            packages localSystem crossSystem));
    };

    packages = forEachSystem (
      pkgs: let
        craneLib = crane.mkLib pkgs;
        commonArgs = {
          src = craneLib.cleanCargoSource ./.;
          strictDeps = true;
          buildInputs = lib.optionals pkgs.stdenv.isDarwin [pkgs.libiconv];
        };
      in {
        default = craneLib.buildPackage (
          commonArgs
          // {
            cargoArtifacts = craneLib.buildDepsOnly commonArgs;
            meta.mainProgram = "steiger";

            propagatedBuildInputs = [pkgs.nix pkgs.nix-eval-jobs];

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
            pkgs.rust-analyzer
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
