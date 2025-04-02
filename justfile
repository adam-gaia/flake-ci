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

ci-fmt:
    treefmt --ci

ci-check:
    cargo lcheck
    cargo lclippy --all-targets -- --deny warnings

ci-build:
    cargo lbuild

ci-test: ci-build
    cargo lbuild --tests
    cargo nextest run

ci-doctests:
    # cargo-nextest doesn't yet support doctests
    # https://github.com/nextest-rs/nextest/issues/16
    cargo ltest --doc

ci: ci-test
