{pkgs, ...}:
derivation {
  name = "foo";
  builder = "${pkgs.bash}/bin/bash";
  args = ["-c" "echo foo > $out"];
  system = builtins.currentSystem;
}
