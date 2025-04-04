{
  flake,
  pkgs,
}:
pkgs.mkShellNoCC {
  packages = [
    flake.packages.${pkgs.system}.default
  ];
}
