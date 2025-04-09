{
  inputs,
  pkgs,
  ...
}: let
  crateBuilder = inputs.self.lib.mkCrateBuilder pkgs;
  commonArgs = crateBuilder.commonArgs;
  cargoArtifacts = crateBuilder.cargoArtifacts;
  craneLib = crateBuilder.craneLib;

  cargo-nextest = craneLib.cargoNextest (commonArgs
    // {
      inherit cargoArtifacts;
      partitions = 1;
      partitionType = "count";
    });
in
  cargo-nextest
