use clap::Parser as _;

const GITHUB_API_URL: &str = "https://api.github.com/repos/rust-lang/rust/releases";

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("Could not fetch Rust releases: {0}")]
    FetchReleases(#[source] ureq::Error),
    #[error("Could not read or decode Rust releases: {0}")]
    DecodeReleases(#[source] ureq::Error),
    #[error("No stable Rust release found")]
    NoStableRelease,
    #[error("Could not read toolchain file {}: {source}", path.display())]
    ReadToolchain {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("Could not parse toolchain TOML {}: {source}", path.display())]
    ParseToolchain {
        path: std::path::PathBuf,
        source: toml_edit::TomlError,
    },
    #[error("'toolchain' must be a table in {}", path.display())]
    InvalidToolchainTable { path: std::path::PathBuf },
    #[error("Could not create toolchain directory {}: {source}", path.display())]
    CreateDirectory {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("Could not write toolchain file {}: {source}", path.display())]
    WriteToolchain {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
}

#[derive(clap::Parser, Debug)]
#[command(
    about = "Update the Rust toolchain to the latest stable release, stripping the patch version"
)]
struct Args {
    #[arg(default_value = "rust-toolchain.toml")]
    toolchain_file: std::path::PathBuf,
}

#[derive(serde::Deserialize, Debug)]
struct Release {
    tag_name: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

fn get_latest_rust_version(url: &str, token: Option<&str>) -> Result<String, Error> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(15)))
        .build()
        .into();
    let mut request = agent
        .get(url)
        .header("User-Agent", "judicious-toolchain-updater")
        .header("Accept", "application/vnd.github+json");
    if let Some(token) = token.filter(|token| !token.is_empty()) {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let releases: Vec<Release> = request
        .call()
        .map_err(Error::FetchReleases)?
        .body_mut()
        .read_json()
        .map_err(Error::DecodeReleases)?;

    for release in releases {
        if release.draft || release.prerelease {
            continue;
        }
        let Some(version) = release
            .tag_name
            .as_deref()
            .and_then(|tag| semver::Version::parse(tag).ok())
        else {
            continue;
        };
        if version.pre.is_empty() && version.build.is_empty() {
            return Ok(format!("{}.{}", version.major, version.minor));
        }
    }
    Err(Error::NoStableRelease)
}

fn update_rust_toolchain_file(path: &std::path::Path, version: &str) -> Result<bool, Error> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(Error::ReadToolchain {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let mut document = contents
        .as_deref()
        .unwrap_or("")
        .parse::<toml_edit::DocumentMut>()
        .map_err(|source| Error::ParseToolchain {
            path: path.to_path_buf(),
            source,
        })?;
    let toolchain = document
        .entry("toolchain")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_like_mut()
        .ok_or_else(|| Error::InvalidToolchainTable {
            path: path.to_path_buf(),
        })?;

    if toolchain.get("channel").and_then(toml_edit::Item::as_str) == Some(version) {
        return Ok(false);
    }

    let mut channel = toml_edit::Value::from(version);
    if let Some(previous) = toolchain.get("channel").and_then(toml_edit::Item::as_value) {
        *channel.decor_mut() = previous.decor().clone();
    }
    toolchain.insert("channel", toml_edit::Item::Value(channel));
    if contents.is_none() {
        toolchain.insert(
            "components",
            toml_edit::Item::Value(toml_edit::Value::Array(toml_edit::Array::new())),
        );
        toolchain.insert(
            "targets",
            toml_edit::Item::Value(toml_edit::Value::Array(toml_edit::Array::new())),
        );
    }

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| Error::CreateDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(path, document.to_string()).map_err(|source| Error::WriteToolchain {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(true)
}

fn run() -> Result<(), Error> {
    let args = Args::parse();
    eprintln!("Fetching latest stable Rust release...");
    let token = std::env::var("GITHUB_TOKEN").ok();
    let version = get_latest_rust_version(GITHUB_API_URL, token.as_deref())?;
    let path = &args.toolchain_file;
    if update_rust_toolchain_file(path, &version)? {
        eprintln!("Updated {} to Rust {version}.", path.display());
    } else {
        eprintln!("{} is already at Rust {version}.", path.display());
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
