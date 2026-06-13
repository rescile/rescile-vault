use clap::Args;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::io::Write;

#[derive(Args, Debug, Clone)]
pub struct UpdateArgs {
    /// The version to update to (e.g., latest, v0.1.0)
    #[arg(default_value = "latest")]
    pub version: String,
}

#[derive(Deserialize, Debug)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize, Debug)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

pub fn handle_update(args: UpdateArgs, exe_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
    const USER_AGENT: &str = "rescile-updater";

    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    if os != "linux" || arch != "x86_64" {
        return Err(format!(
            "Updates are currently only supported for x86_64 Linux. Found OS: {}, ARCH: {}",
            os, arch
        )
        .into());
    }

    println!("Starting update process...");

    let asset_name = if cfg!(feature = "tpm") {
        "rescile-vault-x86_64-linux-gnu-tpm"
    } else {
        "rescile-vault-x86_64-linux-musl"
    };

    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .build()?;

    let release_url = if args.version == "latest" {
        "https://api.github.com/repos/rescile/rescile-vault/releases/latest".to_string()
    } else {
        let tag = if !args.version.starts_with('v') {
            format!("v{}", args.version)
        } else {
            args.version.clone()
        };
        format!(
            "https://api.github.com/repos/rescile/rescile-vault/releases/tags/{}",
            tag
        )
    };

    println!("Fetching release info from GitHub API...");
    let response = client.get(&release_url).send()?.error_for_status()?;
    let release: GithubRelease = response.json()?;

    let target_version = release.tag_name.trim_start_matches('v');

    println!(
        "Requested version '{}' resolved to target version '{}'. Current version is '{}'.",
        args.version, target_version, CURRENT_VERSION
    );

    if target_version == CURRENT_VERSION {
        if args.version == "latest" {
            println!(
                "You are already running the latest version ({}). No update needed.",
                CURRENT_VERSION
            );
        } else {
            println!(
                "You are already running the requested version ({}). No update needed.",
                CURRENT_VERSION
            );
        }
        return Ok(());
    }

    let download_url = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .map(|a| &a.browser_download_url)
        .ok_or_else(|| {
            format!(
                "Asset '{}' not found in release '{}'",
                asset_name, release.tag_name
            )
        })?;

    let checksums_url = release
        .assets
        .iter()
        .find(|a| a.name == "checksums.txt")
        .map(|a| &a.browser_download_url);

    let mut expected_checksum = None;
    if let Some(url) = checksums_url {
        println!("Fetching checksums...");
        let checksums_text = client.get(url).send()?.error_for_status()?.text()?;
        for line in checksums_text.lines() {
            if line.contains(asset_name) {
                expected_checksum = line.split_whitespace().next().map(|s| s.to_string());
                break;
            }
        }
    }

    println!(
        "Downloading new version '{}' for '{}' from: {}",
        target_version, asset_name, download_url
    );
    let binary_data = client
        .get(download_url)
        .send()?
        .error_for_status()?
        .bytes()?;
    let len = binary_data.len();
    let human_readable = if len >= 1_073_741_824 {
        format!("{:.2} GB", len as f64 / 1_073_741_824.0)
    } else if len >= 1_048_576 {
        format!("{:.2} MB", len as f64 / 1_048_576.0)
    } else if len >= 1024 {
        format!("{:.2} KB", len as f64 / 1024.0)
    } else {
        format!("{} bytes", len)
    };
    println!("Download complete ({}).", human_readable);

    if let Some(expected) = expected_checksum {
        println!("Verifying checksum...");
        let mut hasher = Sha256::new();
        hasher.update(&binary_data);
        let calculated = hasher
            .finalize()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>();
        if calculated != expected {
            return Err(format!(
                "Checksum verification failed! Downloaded file is corrupt. Expected: {}, Got: {}",
                expected, calculated
            )
            .into());
        }
        println!("Checksum verified successfully.");
    } else {
        println!("No checksums.txt found for asset, skipping verification.");
    }

    let current_exe = env::current_exe()?;
    let update_dir = current_exe
        .parent()
        .ok_or("Could not determine parent directory of executable")?;
    let exe_filename = current_exe
        .file_name()
        .unwrap_or_default()
        .to_str()
        .unwrap_or(exe_name);

    let new_exe_path = update_dir.join(format!(".{}.new", exe_filename));

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut new_exe_file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o755)
            .open(&new_exe_path)?;
        new_exe_file.write_all(&binary_data)?;
    }

    #[cfg(not(unix))]
    {
        let mut new_exe_file = fs::File::create(&new_exe_path)?;
        new_exe_file.write_all(&binary_data)?;
    }

    println!("New binary written to: {:?}", new_exe_path);

    let old_exe_path = update_dir.join(format!(".{}.old", exe_filename));
    if old_exe_path.exists() {
        let _ = fs::remove_file(&old_exe_path);
    }

    fs::rename(&current_exe, &old_exe_path)?;
    println!("Current binary moved to: {:?}", old_exe_path);

    println!("Moving new binary into place at: {:?}", current_exe);
    if let Err(e) = fs::rename(&new_exe_path, &current_exe) {
        println!("Failed to move new binary, rolling back...");
        let _ = fs::rename(&old_exe_path, &current_exe);
        return Err(e.into());
    }

    if let Err(e) = fs::remove_file(&old_exe_path) {
        println!(
            "Could not remove old binary at {:?}: {}. This can be ignored or manually cleaned up.",
            old_exe_path, e
        );
    }

    println!("Update to version '{}' successful!", args.version);

    Ok(())
}
