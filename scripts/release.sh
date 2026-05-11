#!/usr/bin/env bash
set -euo pipefail

# Release a new version of judicious to crates.io.
#
# Assumes:
#   - cargo, cargo-edit, git are on PATH (e.g., inside `nix develop`).
#   - git is configured with a usable user.name and user.email.
#   - CARGO_REGISTRY_TOKEN is set (or `cargo login` has been run).
#   - The checked-out commit is the one to release (typically the tip of
#     `main`), and the working tree is clean.

repo_root="$(git rev-parse --show-toplevel)"
cd "${repo_root}"

remote="$(git config --get branch.main.remote || true)"
if [[ -z "${remote}" ]]; then
    remote_count="$(git remote | wc -l)"
    if [[ "${remote_count}" -ne 1 ]]; then
        echo "ERROR: cannot determine remote: branch.main.remote is unset and there are ${remote_count} remotes" >&2
        exit 1
    fi
    remote="$(git remote)"
fi

new_version="$("${repo_root}/scripts/compute-next-version-number.sh")"
echo "Releasing version: ${new_version}"

cargo set-version "${new_version}"
cargo update --package judicious

cargo publish --dry-run --allow-dirty

git add Cargo.toml Cargo.lock
git commit --message ":bookmark: Release ${new_version}"

git push "${remote}" HEAD:main

tag="v${new_version}"
git tag --annotate "${tag}" --message "Release ${new_version}"
git push "${remote}" "${tag}"

cargo publish
