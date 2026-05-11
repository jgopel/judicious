#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cargo_toml="${script_dir}/../Cargo.toml"

current_version="$(grep -E '^version = "' "${cargo_toml}" | head -1 | sed -E 's/version = "(.+)"/\1/')"

if [[ ! "${current_version}" =~ ^(20[0-9]{2})\.([1-9]|1[0-2])\.([0-9]+)$ ]]; then
    echo "ERROR: ${cargo_toml} version '${current_version}' does not match YYYY.M.N" >&2
    exit 1
fi

current_year="${BASH_REMATCH[1]}"
current_month="${BASH_REMATCH[2]}"
current_hotfix="${BASH_REMATCH[3]}"

now_year="$(date -u +%Y)"
now_month="$(date -u +%m)"
now_month="${now_month#0}"

if [[ "${current_year}" == "${now_year}" && "${current_month}" == "${now_month}" ]]; then
    new_hotfix=$((current_hotfix + 1))
    new_version="${now_year}.${now_month}.${new_hotfix}"
else
    new_version="${now_year}.${now_month}.0"
fi

echo "${new_version}"
