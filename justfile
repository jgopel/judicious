[parallel]
all: test quality check-all-features udeps

test:
    cargo nextest run --all-targets --all-features

quality:
    prek run --all-files

check-all-features:
    cargo hack check --feature-powerset --all-targets

udeps:
    cargo-nightly hack udeps --feature-powerset --all-targets
