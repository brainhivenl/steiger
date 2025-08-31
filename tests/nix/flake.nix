{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    steiger.url = "github:brainhivenl/steiger";
  };

  outputs = {
    nixpkgs,
    steiger,
    ...
  }: let
    system = "x86_64-linux";
    overlays = [steiger.overlays.ociTools];
    pkgs = import nixpkgs { inherit system overlays; };
  in {
    packages.${system} = {
      default = pkgs.ociTools.buildImage {
        name = "hello";

        copyToRoot = pkgs.buildEnv {
          name = "hello-env";
          paths = [pkgs.hello];
          pathsToLink = ["/bin"];
        };

        config.Cmd = ["/bin/hello"];
        compressor = "none";
      };
    };

    devShells.${system} = {
      default = pkgs.mkShell {
        packages = [steiger.packages.${system}.default];
      };
    };
  };
}
