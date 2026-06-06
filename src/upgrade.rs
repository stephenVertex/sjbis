//! Self-upgrade support: query the GitHub Releases API, compare the running
//! version against the latest tag, then download and atomically replace the
//! current executable with the matching release asset.
//!
//! Release assets are produced by `.github/workflows/release.yml` and named by
//! Rust target triple, e.g. `sjbis-x86_64-apple-darwin.tar.gz`.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::io::Read;

const REPO: &str = "stephenVertex/sjbis";
const USER_AGENT: &str = concat!("sjbis/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// The Rust target triple this binary should download an asset for.
fn target_triple() -> Result<&'static str> {
    // Only the targets the release workflow actually builds are supported.
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-musl"),
        (os, arch) => Err(anyhow!(
            "no prebuilt release for this platform ({os}/{arch}); \
             builds are published for macOS Intel and Linux Intel only"
        )),
    }
}

/// Normalise a version-ish string for comparison: strip a leading `v` and any
/// build metadata after `+` (the git hash we append at build time).
fn normalize(v: &str) -> &str {
    let v = v.strip_prefix('v').unwrap_or(v);
    v.split('+').next().unwrap_or(v)
}

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .context("failed to build HTTP client")
}

async fn fetch_release(client: &reqwest::Client, tag: &Option<String>) -> Result<Release> {
    let url = match tag {
        Some(t) => format!("https://api.github.com/repos/{REPO}/releases/tags/{t}"),
        None => format!("https://api.github.com/repos/{REPO}/releases/latest"),
    };
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("failed to reach the GitHub releases API")?;

    if !resp.status().is_success() {
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(anyhow!(
                "no matching release found on GitHub ({REPO}). \
                 {}",
                match tag {
                    Some(t) => format!("Tag '{t}' does not exist or has no published release."),
                    None => "The repository has no published releases yet.".to_string(),
                }
            ));
        }
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("GitHub API returned {status}: {body}"));
    }
    resp.json::<Release>()
        .await
        .context("failed to parse the GitHub release response")
}

/// `sjbis upgrade` entry point.
pub async fn run(check: bool, force: bool, tag: Option<String>) -> Result<()> {
    let current = crate::version::PKG_VERSION;
    let client = http_client()?;

    println!("Current version: {} ({})", current, crate::version::full());
    if tag.is_some() {
        println!("Requesting release: {}", tag.as_deref().unwrap());
    } else {
        println!("Checking GitHub for the latest release…");
    }

    let release = fetch_release(&client, &tag).await?;
    let latest = normalize(&release.tag_name).to_string();
    println!("Latest release:  {}", release.tag_name);

    let up_to_date = normalize(current) == latest;
    if up_to_date && !force {
        println!("✓ Already up to date.");
        return Ok(());
    }

    if check {
        if up_to_date {
            println!("✓ Already up to date.");
        } else {
            println!(
                "↑ A newer version is available: {} → {}",
                current, release.tag_name
            );
            if !release.html_url.is_empty() {
                println!("  {}", release.html_url);
            }
            println!("  Run `sjbis upgrade` to install it.");
        }
        return Ok(());
    }

    // From here on we actually need a matching prebuilt asset.
    let triple = target_triple()?;

    // Find the asset matching this platform.
    let wanted = format!("sjbis-{triple}.tar.gz");
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == wanted)
        .ok_or_else(|| {
            anyhow!(
                "release {} has no asset named '{}' (available: {})",
                release.tag_name,
                wanted,
                release
                    .assets
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;

    println!("Downloading {} …", asset.name);
    let bytes = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .context("failed to download the release asset")?
        .error_for_status()
        .context("release asset download returned an error status")?
        .bytes()
        .await
        .context("failed to read the release asset body")?;

    let new_binary = extract_binary(&bytes).context("failed to extract sjbis from the archive")?;

    // Write to a temp file beside the current exe, mark executable, then swap.
    let current_exe = std::env::current_exe().context("cannot locate the current executable")?;
    let dir = current_exe
        .parent()
        .ok_or_else(|| anyhow!("current executable has no parent directory"))?;

    let tmp = dir.join(format!(".sjbis-upgrade-{}", std::process::id()));
    std::fs::write(&tmp, &new_binary)
        .with_context(|| format!("failed to write new binary to {}", tmp.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&tmp)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&tmp, perms)?;
    }

    // self_replace handles the case where the running binary is the target.
    self_replace::self_replace(&tmp)
        .with_context(|| format!("failed to replace {}", current_exe.display()))?;
    let _ = std::fs::remove_file(&tmp);

    println!(
        "✓ Upgraded {} → {} ({})",
        current,
        release.tag_name,
        current_exe.display()
    );
    println!("  Restart any running `sjbis daemon` for the new version to take effect.");
    Ok(())
}

/// Pull the `sjbis` entry out of a gzipped tarball.
fn extract_binary(gz: &[u8]) -> Result<Vec<u8>> {
    let decoder = flate2::read::GzDecoder::new(gz);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        let is_bin = path
            .file_name()
            .map(|n| n == "sjbis")
            .unwrap_or(false)
            && path.components().count() == 1;
        if is_bin {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            return Ok(buf);
        }
    }
    Err(anyhow!("archive did not contain a top-level 'sjbis' binary"))
}
