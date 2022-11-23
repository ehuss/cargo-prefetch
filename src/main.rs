use anyhow::{anyhow, bail, format_err, Context, Result};
use clap::{crate_version, App, AppSettings, Arg, SubCommand};
use serde_derive::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    process::Command,
};
use tempfile::TempDir;
use toml::Value;

mod top;

const TEMP_PROJ_NAME: &str = "temp_prefetch_project";

const HELP: &str = "\
This command is used to download some popular dependencies into Cargo's cache. \
This is useful if you plan to go offline, and you want a collection of common \
crates available to use.

By default, if no options are given, it will download the top 100 most used \
dependencies (--top-deps=100).
";
#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("Error: {}", e);
        for cause in e.chain() {
            eprintln!("Caused by: {}", cause);
        }
        std::process::exit(1);
    }
}

type CrateSet = HashMap<String, HashSet<String>>;

async fn run() -> Result<()> {
    let app_matches = App::new("cargo-prefetch")
        .version(crate_version!())
        .bin_name("cargo")
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .global_settings(&[
            AppSettings::GlobalVersion, // subcommands inherit version
            AppSettings::ColoredHelp,
            AppSettings::DeriveDisplayOrder,
        ])
        .subcommand(
            SubCommand::with_name("prefetch")
                .about("Download popular crates.")
                .after_help(HELP)
                .arg(
                    Arg::with_name("list")
                        .long("list")
                        .help("List what is downloaded instead of downloading."),
                )
                .arg(
                    Arg::with_name("verbose")
                        .short("v")
                        .long("verbose")
                        .help("Print some extra info to stderr."),
                )
                .arg(
                    Arg::with_name("top-deps")
                        .long("top-deps")
                        .min_values(0)
                        .max_values(1)
                        .help(
                            "Download the most frequent dependencies. \
                             Specify a value for the number to download, default is 100.",
                        ),
                )
                .arg(
                    Arg::with_name("top-downloads")
                        .long("top-downloads")
                        .min_values(0)
                        .max_values(1)
                        .help(
                            "Download the most downloaded crates. \
                             Specify a value for the number to download, default is 100.",
                        ),
                )
                .arg(
                    Arg::with_name("lockfile")
                        .long("lockfile")
                        .takes_value(true)
                        .help("Download all crates listed in the specified lockfile."),
                )
                .arg(Arg::with_name("crates").multiple(true).help(
                    "Specify individual crates to download. \
                     Use the syntax `crate_name@=2.7.0` to download a specific version.",
                )),
        )
        .get_matches();

    let matches = app_matches
        .subcommand_matches("prefetch")
        .expect("Expected `prefetch` subcommand.");

    let verbose = matches.is_present("verbose");

    let parse_int = |name: &str| match matches.value_of(name) {
        Some(value) => match value.parse::<usize>() {
            Ok(v) => Ok(Some(v)),
            Err(e) => bail!("{} must be an integer: {}", name, e),
        },
        None => {
            if matches.is_present(name) {
                Ok(Some(100))
            } else {
                Ok(None)
            }
        }
    };

    let mut top_deps = parse_int("top-deps")?;
    let top_downloads = parse_int("top-downloads")?;

    // Default behavior with no command-line options.
    if !matches.is_present("crates")
        && !matches.is_present("lockfile")
        && top_deps.is_none()
        && top_downloads.is_none()
    {
        top_deps = Some(100);
    }

    let mut crates: CrateSet = HashMap::new();
    if let Some(top) = top_deps {
        for name in top::TOP_CRATES.iter().take(top) {
            crates.entry(name.to_string()).or_insert_with(HashSet::new);
        }
    }
    if let Some(top) = top_downloads {
        for name in top_crates_io(verbose, top).await? {
            crates.entry(name.to_string()).or_insert_with(HashSet::new);
        }
    }

    if let Some(explicit_crates) = matches.values_of("crates") {
        for krate in explicit_crates {
            let mut splits = krate.split('@');
            let name = splits.next().ok_or_else(|| format_err!("empty argument"))?;
            let version = splits.next().map(|s| s.to_string());
            crates
                .entry(name.to_string())
                .or_insert_with(HashSet::new)
                // add the version if present
                .extend(version.into_iter().collect::<Vec<_>>());
        }
    }

    if let Some(lockfile) = matches.value_of("lockfile") {
        parse_lockfile(Path::new(lockfile), &mut crates)?;
    }

    if matches.is_present("list") {
        list(verbose, &crates)
    } else {
        if verbose {
            list(verbose, &crates)?;
        }
        do_fetch(verbose, &crates)
    }
}

/// Perform the download.
fn do_fetch(verbose: bool, crates: &CrateSet) -> Result<()> {
    let dir = mktemp()?;
    let tmp_path = dir.path();
    make_project(tmp_path, crates)?;

    if verbose {
        eprintln!("Running: cargo fetch");
    }

    let status = Command::new("cargo")
        .arg("fetch")
        .current_dir(tmp_path)
        .status()
        .with_context(|| "Failed to launch `cargo`.")?;
    if !status.success() {
        bail!("`cargo` failed to run: {}", status);
    }

    Ok(())
}

/// Print all packages that would be downloaded.
fn list(verbose: bool, crates: &CrateSet) -> Result<()> {
    let dir = mktemp()?;
    let tmp_path = dir.path();
    make_project(tmp_path, crates)?;
    if verbose {
        eprintln!("Running: cargo generate-lockfile");
    }
    let output = Command::new("cargo")
        .arg("generate-lockfile")
        .current_dir(tmp_path)
        .output()
        .with_context(|| "Failed to launch `cargo`.")?;
    if !output.status.success() {
        bail!(
            "`cargo` failed to run:\n{}\n{}\n{}\n",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let pkgs = load_from_lock(tmp_path)?;
    for pkg in pkgs {
        if pkg.name != TEMP_PROJ_NAME {
            println!("{} = \"{}\"", pkg.name, pkg.version);
        }
    }
    Ok(())
}

/// Create a temporary Cargo project with the given dependencies.
fn make_project(tmp_path: &Path, crates: &CrateSet) -> Result<()> {
    let invalid_pkg_name_chars = regex::Regex::new("[^-_0-9a-zA-Z]").unwrap();
    let deps: Vec<String> = crates
        .iter()
        .map(|(name, versions)| {
            if versions.is_empty() {
                // use newest
                format!("\"{}\" = \"*\"\n", name,)
            } else {
                versions
                    .iter()
                    .map(|v| {
                        // combine name and version for pkg alias to allow multiple versions
                        format!(
                            "\"{}__{}\" = {{ package = \"{}\", version = \"{}\" }}\n",
                            name,
                            invalid_pkg_name_chars.replace_all(v, "_"),
                            name,
                            v
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("")
            }
        })
        .collect();

    // NOTE: This method of using a single project to resolve all dependencies
    // may result in some crates using an older version due to restrictive
    // version requirements. In practice I haven't seen any that are forced to
    // resolve to an older version.

    fs::write(
        tmp_path.join("Cargo.toml"),
        format!(
            r#"
            [package]
            name = "{}"
            version = "0.0.0"

            [dependencies]
            {}
            "#,
            TEMP_PROJ_NAME,
            deps.join("")
        ),
    )?;
    fs::create_dir(tmp_path.join("src"))?;
    fs::write(tmp_path.join("src").join("lib.rs"), "")?;
    Ok(())
}

fn mktemp() -> Result<TempDir> {
    tempfile::tempdir().with_context(|| "Failed to create temp directory.")
}

#[derive(Deserialize)]
struct Package {
    name: String,
    version: String,
}

#[derive(Deserialize)]
struct Lockfile {
    package: Option<Vec<Package>>,
}

#[derive(Deserialize)]
struct CratesQuery {
    crates: Vec<CrateInfo>,
}

#[derive(Deserialize)]
struct CrateInfo {
    name: String,
}

/// Load a list of packages from a Cargo.lock file.
fn load_from_lock(dir: &Path) -> Result<Vec<Package>> {
    let contents = fs::read_to_string(dir.join("Cargo.lock"))?;
    let lock: Lockfile = toml::from_str(&contents)?;
    Ok(lock.package.unwrap_or_default())
}

/// Return the top downloaded crates by querying crates.io.
async fn top_crates_io(verbose: bool, mut count: usize) -> Result<Vec<String>> {
    const CRATES_IO_MAX: usize = 100;
    let mut result = Vec::new();
    let mut page = 1;
    let client = reqwest::Client::new();

    while count > 0 {
        let n = count.min(CRATES_IO_MAX);
        let q = format!(
            "https://crates.io/api/v1/crates?page={}&per_page={}&sort=downloads",
            page, n
        );
        if verbose {
            eprintln!("Sending request: {}", q);
        }
        let response = client
            .get(&q)
            // crates.io requires a meaningful user agent
            .header(
                "User-Agent",
                "cargo-prefetch (https://github.com/ehuss/cargo-prefetch)",
            )
            .send()
            .await
            .map_err(|e| anyhow!(e))
            .with_context(|| "Failed to fetch top crates from crates.io.")?;
        let status = response.status();
        if !status.is_success() {
            let headers: Vec<_> = response
                .headers()
                .iter()
                .map(|(key, value)| format!("{}: {:?}", key, value))
                .collect();
            bail!(
                "Failed to fetch top crates crom crates.io.\n\
                Status: {}\n\
                Headers:\n\
                {}\n\
                {}
                ",
                status,
                headers.join("\n"),
                response.text().await.unwrap_or_else(|e| format!("{:?}", e))
            );
        }

        let json: CratesQuery = response.json().await?;
        for c in json.crates.into_iter() {
            result.push(c.name);
        }
        page += 1;
        count -= n;
    }
    Ok(result)
}

fn parse_lockfile(lockfile: &Path, crates: &mut CrateSet) -> Result<()> {
    let toml_value = fs::read_to_string(lockfile)?.parse::<Value>()?;
    let packages = toml_value
        .as_table()
        .and_then(|t| t.get("package").and_then(|value| value.as_array()))
        .and_then(|packages| {
            packages
                .iter()
                .map(|value| value.as_table())
                .collect::<Option<Vec<_>>>()
        })
        .ok_or_else(|| anyhow!("Unexpected toml structure"))?;

    for pkg in packages {
        // only look for crates.io packages
        if !(pkg.get("source").and_then(|v| v.as_str())
            == Some("registry+https://github.com/rust-lang/crates.io-index"))
        {
            continue;
        }
        let name = pkg
            .get("name")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .ok_or_else(|| anyhow!("Missing package name"))?;
        let version = pkg
            .get("version")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .ok_or_else(|| anyhow!("Missing package version"))?;

        crates
            .entry(name.to_string())
            .or_insert_with(HashSet::new)
            .insert(version);
    }

    Ok(())
}
