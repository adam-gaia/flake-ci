{pkgs, ...}:
derivation {
  name = "bar";
  builder = "${pkgs.bash}/bin/bash";
  args = ["-c" "echo bar > $out"];
  system = builtins.currentSystem;
}
