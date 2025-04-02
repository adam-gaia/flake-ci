{inputs, ...}: let
  mkCrateBuilder = pkgs: let
    inherit (pkgs) lib;
    craneLib = inputs.crane.mkLib pkgs;
    txtFilter = path: _type: builtins.match ".*txt" path != null;
    markdownFilter = path: _type: builtins.match ".*md" path != null;
    sourceWithReadme = path: type: (markdownFilter path type) || (txtFilter path type) || (craneLib.filterCargoSources path type);
    # nix build needs access to the READAME and test/*.txt files for trycmd tests
    src = lib.cleanSourceWith {
      src = ../../.;
      filter = sourceWithReadme;
      name = "source";
    };

    # Common arguments can be set here to avoid repeating them later
    commonArgs = {
      inherit src;
      strictDeps = true;
      pname = "flake";

      buildInputs =
        [
          # Add additional build inputs here
        ]
        ++ lib.optionals pkgs.stdenv.isDarwin [
          # Additional darwin specific inputs can be set here
          pkgs.libiconv
        ];

      # Additional environment variables can be set directly
      # MY_CUSTOM_VAR = "some value";
    };
  in {
    inherit src lib craneLib commonArgs;
  };
in {
  inherit mkCrateBuilder;
}
