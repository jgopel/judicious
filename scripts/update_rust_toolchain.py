#!/usr/bin/env python3
"""Update the Rust toolchain file to the latest stable release."""

import argparse
import logging
import os
import sys
from pathlib import Path

import requests
import toml

logger = logging.getLogger(__name__)

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - %(levelname)s - %(message)s",
    datefmt="%Y-%m-%d %H:%M:%S",
)

GITHUB_API_URL = "https://api.github.com/repos/rust-lang/rust/releases"


def get_latest_rust_version() -> str | None:
    try:
        logger.info("Fetching latest Rust releases from GitHub...")
        headers = {}
        token = os.environ.get("GITHUB_TOKEN")
        if token:
            headers["Authorization"] = f"Bearer {token}"
        response = requests.get(GITHUB_API_URL, headers=headers, timeout=15)
        response.raise_for_status()

        releases = response.json()

        if not releases:
            logger.error("No releases found in the API response.")
            return None

        for release in releases:
            tag_name = release.get("tag_name")
            if isinstance(tag_name, str) and not any(
                suffix in tag_name for suffix in ["-beta", "-nightly", "-alpha", "rc"]
            ):
                logger.info("Found latest stable Rust version: %s", tag_name)
                return tag_name

        logger.error(
            "Could not find a suitable stable release tag_name in the API response.",
        )
    except requests.exceptions.Timeout:
        logger.exception("Timeout while trying to connect to %s", GITHUB_API_URL)
        return None
    except requests.exceptions.RequestException:
        logger.exception("Error fetching data from GitHub")
        return None
    except ValueError:
        logger.exception("Error parsing JSON response")
        return None
    else:
        return None


def update_rust_toolchain_file(
    toolchain_file_path: Path,
    latest_version: str,
) -> bool:
    toolchain_data = {}
    if not toolchain_file_path.exists():
        logger.info("%s not found. Creating a new file.", toolchain_file_path)
        toolchain_data = {
            "toolchain": {"channel": latest_version, "components": [], "targets": []},
        }
    else:
        try:
            logger.info("Reading %s...", toolchain_file_path)
            with toolchain_file_path.open("r", encoding="utf-8") as f:
                toolchain_data = toml.load(f)
        except toml.TomlDecodeError:
            logger.exception("Error decoding TOML from %s", toolchain_file_path)
            return False
        except OSError:
            logger.exception("Error reading %s", toolchain_file_path)
            return False

        if "toolchain" not in toolchain_data:
            logger.warning(
                "'[toolchain]' section not found in %s. Adding it.",
                toolchain_file_path,
            )
            toolchain_data["toolchain"] = {}

        current_version = toolchain_data["toolchain"].get("channel")

        if current_version == latest_version:
            logger.info(
                "Rust toolchain at %s is already up to date (Version: %s).",
                toolchain_file_path,
                current_version,
            )
            return True

        logger.info(
            "Updating toolchain channel in %s from '%s' to '%s'.",
            toolchain_file_path,
            current_version,
            latest_version,
        )
        toolchain_data["toolchain"]["channel"] = latest_version

    try:
        logger.info("Writing updated configuration to %s...", toolchain_file_path)
        toolchain_file_path.parent.mkdir(parents=True, exist_ok=True)
        with toolchain_file_path.open("w", encoding="utf-8") as f:
            toml.dump(toolchain_data, f)
        logger.info("%s updated successfully.", toolchain_file_path)
    except OSError:
        logger.exception("Error writing to %s", toolchain_file_path)
        return False
    else:
        return True


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Update the Rust toolchain file (e.g., rust-toolchain.toml) to the latest stable version, stripping patch version.",
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )
    parser.add_argument(
        "toolchain_file",
        type=Path,
        nargs="?",
        default=Path("rust-toolchain.toml"),
        help="Path to the rust-toolchain.toml file (or rust-toolchain).",
    )
    args = parser.parse_args()

    toolchain_file_path: Path = args.toolchain_file.resolve()

    logger.info("--- Rust Toolchain Updater ---")
    logger.info("Targeting toolchain file: %s", toolchain_file_path)

    latest_version_full = get_latest_rust_version()

    if latest_version_full:
        version_parts = latest_version_full.split(".")
        processed_version: str
        min_version_parts = 3
        if len(version_parts) >= min_version_parts:
            processed_version = f"{version_parts[0]}.{version_parts[1]}"
            logger.info(
                "Original latest version: %s, Processed version for toolchain (stripping patch): %s",
                latest_version_full,
                processed_version,
            )
        else:
            processed_version = latest_version_full
            logger.info(
                "Latest version (%s) does not have a patch component or is shorter; using as is.",
                latest_version_full,
            )

        if update_rust_toolchain_file(toolchain_file_path, processed_version):
            logger.info("Update process completed successfully.")
            sys.exit(0)
        else:
            logger.error("Update process failed.")
            sys.exit(1)
    else:
        logger.error("Could not retrieve the latest Rust version. Aborting update.")
        sys.exit(1)


if __name__ == "__main__":
    main()
