{rustPlatform}:
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "nix2oci";
  version = "0.1.0";

  src = ./.;
  cargoLock = {
    lockFile = ./Cargo.lock;
  };

  doCheck = false;
  meta.mainProgram = "nix2oci";
})
