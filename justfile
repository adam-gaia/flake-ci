system := `system-str`

default:
    @just --list

check:
    cargo lcheck
    cargo lclippy

build:
    cargo lbuild

run:
    RUST_LOG=debug cargo lrun

test: build
    cargo lbuild --tests
    cargo nextest run --all-targets

fmt:
    treefmt

ci:
    # TODO: probably need to not use cachix when running locally. Maybe disable cachix if secret api key env var not set
    flake-ci
