{
  lib,
  stdenvNoCC,
  runCommand,
  writeClosure,
  closureInfo,
  steiger,
}: let
  defaultArch = let
    inherit (stdenvNoCC.targetPlatform) system;
  in (
    if lib.hasPrefix "aarch64-" system
    then "arm64"
    else if lib.hasPrefix "x86_64-" system
    then "amd64"
    else throw "unsupported system: ${system}"
  );

  joinLines = lib.concatStringsSep "\n";

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

  # writeClosure wrapper which allows direct closure passthrough
  writeLayerClosure = layer:
    if layer ? closure
    then layer.closure
    else writeClosure layer;

  buildImage = {
    name,
    tag ? "latest",
    layers,
    pathsToLink ? null,
    config ? {},
    os ? "linux",
    arch ? defaultArch,
  }:
    runCommand name
    {nativeBuildInputs = [steiger];}
    # sh
    ''
      steiger nix-to-oci \
          ${joinLines (map (closure: "--closure=${closure} \\") (map writeLayerClosure layers))}
          ${joinLines (map (path: "--link-path=${path} \\") pathsToLink)}
          --out-path $out \
          --name='${name}' \
          --tag='${tag}' \
          --os='${os}' \
          --arch='${arch}' \
          --config='${builtins.toJSON config}'
    '';
in {
  inherit splitDeps buildImage;
}
