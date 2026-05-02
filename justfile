all: test quality check-all-features udeps

test:
    cargo nextest run --all-targets --all-features

quality:
    uv run pre-commit run --all-files

check-all-features:
    RUSTFLAGS=-Awarnings cargo hack check --feature-powerset --all-targets

udeps:
    RUSTFLAGS=-Awarnings cargo-nightly hack udeps --feature-powerset --all-targets
