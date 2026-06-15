# Development Guide

This project combines Rust for the core library with Python tooling for
development and quality checks.

## Prerequisites

- Rust: [Install Rust](https://www.rust-lang.org/tools/install)
- Python: [Install Python](https://www.python.org/downloads/)
- uv: [Install uv](https://docs.astral.sh/uv/getting-started/installation/)
  (used for managing dev dependencies)

## Setup

1. Install Python dependencies:
   ```sh
   uv sync
   ```

## Common Tasks

The project uses a `justfile` to coordinate common tasks.

### Running Tests

To run the full Rust test suite:

```sh
just test
# OR directly via cargo
cargo test --all-targets --all-features
```

### Code Quality & Linting

To run all code quality checks (formatting, linting, etc.):

```sh
just quality
```

This command executes `pre-commit` across all files. It runs:

- **General**: YAML/TOML checks, trailing whitespace, etc.
- **Rust**: `cargo fmt`, `cargo check`, `cargo clippy`, `cargo machete` (unused
  dependency check), and `cargo-sort`.

**Note:** You do not need to install the git hooks locally to run these checks;
`just quality` runs them on demand.

### Commit Messages

Please write your commits in a way that conforms to the commit template. You can
commit with the template by running

```
git commit -t COMMIT_MESSAGE_TEMPLATE
```

or install it so that it's always used automatically within this repo with

```
git config commit.template COMMIT_MESSAGE_TEMPLATE
```

If you have not read them, please make sure to read and follow
https://cbea.ms/git-commit/ and https://conventionalcomments.org/.
