{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    nix-filter.url = "github:numtide/nix-filter";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nix-filter,
    nixpkgs,
    crane,
    ...
  }: let
    inherit (nixpkgs) lib;
    filter = nix-filter.lib;

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
        src = filter {
          root = ./.;
          include = [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };
        commonArgs = {
          inherit src;
          strictDeps = true;
          buildInputs = lib.optionals pkgs.stdenv.isDarwin [pkgs.libiconv];
        };

        steiger = craneLib.buildPackage (
          commonArgs
          // {
            cargoArtifacts = craneLib.buildDepsOnly commonArgs;
            meta.mainProgram = "steiger";

            propagatedBuildInputs = [pkgs.nix pkgs.nix-eval-jobs];

            NIX_BINARY = lib.getExe pkgs.nix;
            NIX_EVAL_JOBS_BINARY = lib.getExe pkgs.nix-eval-jobs;
          }
        );
      in {
        default = steiger;
        ociTools = pkgs.callPackage ./oci-tools {};
      }
    );

    checks = forEachSystem (pkgs: {
      inherit (self.packages.${pkgs.stdenv.hostPlatform.system}) default;
    });

    devShells = forEachSystem (
      pkgs: {
        default = (crane.mkLib pkgs).devShell {
          packages = [
            pkgs.rust-analyzer
            pkgs.nix-eval-jobs
          ];
        };
        steiger = pkgs.mkShell {
          packages = [self.packages.${pkgs.stdenv.hostPlatform.system}.default];
        };
      }
    );

    overlays = {
      default = final: prev: {
        steiger = self.packages.${final.stdenv.hostPlatform.system}.default;
        ociTools = self.packages.${final.stdenv.hostPlatform.system}.ociTools;
      };
      ociTools = final: prev: {
        ociTools = self.packages.${final.stdenv.hostPlatform.system}.ociTools;
      };
    };
  };
}
