{pkgs, ...}:
derivation {
  name = "test";
  builder = "${pkgs.bash}/bin/bash";
  args = ["-c" "echo Hello, World! > $out"];
  system = builtins.currentSystem;
}
