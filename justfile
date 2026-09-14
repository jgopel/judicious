[parallel]
all: test quality check-all-features udeps

test:
    cargo nextest run --workspace --all-targets --all-features

quality:
    prek run --all-files

check-all-features:
    cargo hack check --workspace --feature-powerset --all-targets

udeps:
    cargo-nightly hack udeps --workspace --feature-powerset --all-targets

update-rust-toolchain path="rust-toolchain.toml":
    cargo xtask --bin update_rust_toolchain -- {{ quote(path) }}
