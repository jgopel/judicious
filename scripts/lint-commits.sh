#!/usr/bin/env bash
set -euo pipefail

# Lint commit messages in a range using commitlint.
#
# Environment variables:
#   BASE   - Override the base ref to lint from (e.g. PR base commit).
#            Defaults to 'main'.
#   LATEST - Override the latest ref to lint to. Defaults to 'HEAD'.

base="${BASE:-main}"
latest="${LATEST:-HEAD}"

git rev-parse --verify "${base}" >/dev/null 2>&1 \
    || { echo "ERROR: ref '${base}' not found"; exit 1; }
git rev-parse --verify "${latest}" >/dev/null 2>&1 \
    || { echo "ERROR: ref '${latest}' not found"; exit 1; }

merge_base="$(git merge-base "${base}" "${latest}")" \
    || { echo "ERROR: could not compute merge-base between '${base}' and '${latest}'"; exit 1; }

commit_count="$(git rev-list --count "${merge_base}..${latest}")"
if [[ "${commit_count}" -eq 0 ]]; then
    echo "No commits in range ${merge_base}..${latest}, nothing to lint"
    exit 0
fi

echo "Linting ${commit_count} commit(s) in range ${merge_base}..${latest}"
commitlint --from "${merge_base}" --to "${latest}" --config commitlint.config.cjs
