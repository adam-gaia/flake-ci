{
  inputs,
  pkgs,
  ...
}: let
  crateBuilder = inputs.self.lib.mkCrateBuilder pkgs;
  commonArgs = crateBuilder.commonArgs;
  craneLib = crateBuilder.craneLib;

  src = commonArgs.src;
  advisory-db = inputs.advisory-db;

  # Run cargo audit
  cargo-audit = craneLib.cargoAudit {
    inherit src advisory-db;
  };
in
  cargo-audit
