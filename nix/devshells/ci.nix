{pkgs}:
pkgs.mkShellNoCC {
  packages = [
    # inputs.self.packages.${pkgs.system}.default
  ];
}
