{
  inputs,
  pkgs,
  ...
}: let
  crateBuilder = inputs.self.lib.mkCrateBuilder pkgs;
  commonArgs = crateBuilder.commonArgs;
  craneLib = crateBuilder.craneLib;

  src = commonArgs.src;

  # Run cargo deny
  cargo-deny = craneLib.cargoDeny {
    inherit src;
  };
in
  cargo-deny
