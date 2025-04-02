{
  inputs,
  pkgs,
  ...
}: let
  crateBuilder = inputs.self.lib.mkCrateBuilder pkgs;
  commonArgs = crateBuilder.commonArgs;
  craneLib = crateBuilder.craneLib;

  # Build *just* the cargo dependencies, so we can reuse
  # all of that work (e.g. via cachix) when running in CI
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;

  # Build the actual crate itself, reusing the dependency
  # artifacts from above.
  crate = craneLib.buildPackage (commonArgs
    // {
      inherit cargoArtifacts;
      doCheck = false; # Don't run tests as part of the build
    });
in
  crate
# // {
#   passthru =
#     my-crate.passthru
#     // {
#       tests = checks;
#     };
# }

