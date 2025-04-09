{
  inputs,
  pkgs,
  ...
}: let
  crateBuilder = inputs.self.lib.mkCrateBuilder pkgs;
  commonArgs = crateBuilder.commonArgs;
  cargoArtifacts = crateBuilder.cargoArtifacts;
  craneLib = crateBuilder.craneLib;

  # Run clippy (and deny all warnings) on the crate source
  cargo-clippy = craneLib.cargoClippy (commonArgs
    // {
      inherit cargoArtifacts;
      cargoClippyExtraArgs = "--all-targets -- --deny warnings";
    });
in
  cargo-clippy
