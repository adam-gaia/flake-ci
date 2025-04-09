{
  inputs,
  pkgs,
  ...
}: let
  treefmt = inputs.self.lib.mkTreefmt pkgs ../treefmt.nix;
  treefmt-bin = treefmt.treefmt-bin;
  treefmt-programs = treefmt.treefmt-programs;
in
  pkgs.runCommand "treefmt" {
    src = inputs.self;
    nativeBuildInputs = [treefmt-bin] ++ treefmt-programs;
  }
  ''
    cd $src
    treefmt --ci
    touch $out
  ''
