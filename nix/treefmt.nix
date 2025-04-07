{
  projectRootFile = "flake.nix";
  programs = {
    alejandra.enable = true; # nix
    deadnix.enable = true; # nix
    rustfmt.enable = true; # rust
    just.enable = true; # justfile
    mdformat.enable = true; # markdown
    jsonfmt.enable = true; # json
    yamlfmt.enable = true; # yaml
    taplo.enable = true; # toml
    #typos.enable = true; # spellcheck # Disabled until https://github.com/numtide/treefmt/issues/198 is fixed
  };
}
