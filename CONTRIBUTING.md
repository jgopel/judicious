# Development Guide

This project uses Rust for the library and maintenance tooling, with prek for
formatting and quality checks.

## Prerequisites

- Nix: [Install Nix](https://nixos.org/download/), with flakes enabled.

## Setup

Enter the development shell to get the pinned Rust toolchain, just, prek, and
Cargo utilities:

```sh
nix develop
```

## Common Tasks

The project uses a `justfile` to coordinate common tasks.

### Running Tests

To run the Rust library and maintenance-tool test suites:

```sh
just test
```

### Code Quality & Linting

To run all code quality checks (formatting, linting, etc.):

```sh
just quality
```

This command executes `prek` across all files. It runs:

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
