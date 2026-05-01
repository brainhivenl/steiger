{
  lib,
  stdenvNoCC,
  runCommand,
  writeClosure,
  closureInfo,
  callPackage,
}: let
  nix2oci = callPackage ./package.nix {};

  defaultArch = let
    inherit (stdenvNoCC.targetPlatform) system;
  in (
    if lib.hasPrefix "aarch64-" system
    then "arm64"
    else if lib.hasPrefix "x86_64-" system
    then "amd64"
    else throw "unsupported system: ${system}"
  );

  # split derivation into 2 seperate closures separating the main package and it's dependencies
  splitDeps = drv: let
    fullClosure = closureInfo {rootPaths = [drv];};
    depsPaths = runCommand "deps-paths" {} ''
      grep -v "${drv}" ${fullClosure}/store-paths > $out
    '';
    drvPaths = runCommand "app-paths" {} ''
      echo "${drv}" > $out
    '';
  in
    map (closure: {inherit closure;}) [depsPaths drvPaths];

  buildImage = {
    name,
    tag ? "latest",
    layers,
    pathsToLink ? [],
    config ? {},
    os ? "linux",
    arch ? defaultArch,
  }: let
    writeLayerClosure = layer:
      if layer ? closure
      then layer.closure
      else writeClosure layer;

    closureArgs = map (closure: "--closure ${closure}") (map writeLayerClosure layers);
    linkPathArgs = map (path: "--link-path ${path}") pathsToLink;
    joinArgs = lib.concatStringsSep " ";
  in
    runCommand name {nativeBuildInputs = [nix2oci];}
    # sh
    ''
      nix2oci ${joinArgs closureArgs} ${joinArgs linkPathArgs} \
          --name '${name}' \
          --tag '${tag}' \
          --out-path $out \
          --os '${os}' \
          --arch '${arch}' \
          --config '${builtins.toJSON config}'
    '';
in {
  inherit splitDeps buildImage;
}
