//! GitHub release update check, download, verification, and binary replacement.

use std::path::PathBuf;
use std::time::Duration;

// ── Types ─────────────────────────────────────────────────────────────────────

/// Result of checking for updates.
pub enum UpdateCheck {
    /// No update available (current version is latest or newer).
    UpToDate,
    /// A newer version is available.
    Available {
        version: String,
        asset_url: String,
        checksum_url: String,
        asset_name: String,
    },
    /// Could not check (network error, API error, parse error).
    Failed(String),
}

/// Result of the full update process.
pub enum UpdateResult {
    /// No update was needed.
    UpToDate,
    /// Update downloaded and ready — caller should restart.
    ReadyToRestart {
        new_exe_path: PathBuf,
        version: String,
    },
    /// Update check or download failed — continue with current version.
    Failed(String),
}

// ── Platform detection ────────────────────────────────────────────────────────

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const PLATFORM_ASSET_NAME: &str = "bursar-linux-x86_64";

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const PLATFORM_ASSET_NAME: &str = "bursar-windows-x86_64.exe";

#[cfg(not(any(
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "windows", target_arch = "x86_64")
)))]
const PLATFORM_ASSET_NAME: &str = "";

// ── GitHub API client ─────────────────────────────────────────────────────────

/// Checks GitHub Releases API for a newer version.
///
/// Returns `UpdateCheck::Available` if a newer version exists,
/// `UpdateCheck::UpToDate` if current, or `UpdateCheck::Failed` on any error.
pub fn check_for_update(github_repo: &str) -> UpdateCheck {
    if PLATFORM_ASSET_NAME.is_empty() {
        return UpdateCheck::Failed("no release asset available for this platform".to_string());
    }

    let url = format!("https://api.github.com/repos/{github_repo}/releases/latest");
    let user_agent = format!("bursar/{}", env!("CARGO_PKG_VERSION"));

    let response = match ureq::get(&url)
        .set("User-Agent", &user_agent)
        .set("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(10))
        .call()
    {
        Ok(r) => r,
        Err(e) => return UpdateCheck::Failed(format!("API request failed: {e}")),
    };

    let json: serde_json::Value = match response.into_json() {
        Ok(j) => j,
        Err(e) => return UpdateCheck::Failed(format!("failed to parse API response: {e}")),
    };

    parse_update_check(&json, env!("CARGO_PKG_VERSION"), PLATFORM_ASSET_NAME)
}

/// Parses a GitHub API JSON response into an `UpdateCheck`.
/// Separated from `check_for_update` for testability.
fn parse_update_check(
    json: &serde_json::Value,
    current_version: &str,
    asset_name: &str,
) -> UpdateCheck {
    let tag = match json["tag_name"].as_str() {
        Some(t) => t,
        None => return UpdateCheck::Failed("missing tag_name in API response".to_string()),
    };
    let remote_str = tag.strip_prefix('v').unwrap_or(tag);

    let local = match semver::Version::parse(current_version) {
        Ok(v) => v,
        Err(e) => return UpdateCheck::Failed(format!("failed to parse local version: {e}")),
    };
    let remote = match semver::Version::parse(remote_str) {
        Ok(v) => v,
        Err(e) => {
            return UpdateCheck::Failed(format!(
                "failed to parse remote version '{remote_str}': {e}"
            ));
        }
    };

    if remote <= local {
        return UpdateCheck::UpToDate;
    }

    let assets = match json["assets"].as_array() {
        Some(a) => a,
        None => return UpdateCheck::Failed("missing assets array in API response".to_string()),
    };

    let asset_url = match find_asset_url(assets, asset_name) {
        Some(url) => url,
        None => return UpdateCheck::Failed(format!("no asset named '{asset_name}' in release")),
    };

    let checksum_url = match find_asset_url(assets, "checksums.txt") {
        Some(url) => url,
        None => return UpdateCheck::Failed("no checksums.txt asset in release".to_string()),
    };

    UpdateCheck::Available {
        version: remote_str.to_string(),
        asset_url,
        checksum_url,
        asset_name: asset_name.to_string(),
    }
}

fn find_asset_url(assets: &[serde_json::Value], name: &str) -> Option<String> {
    assets.iter().find_map(|a| {
        if a["name"].as_str() == Some(name) {
            a["browser_download_url"].as_str().map(str::to_string)
        } else {
            None
        }
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_api_response() -> serde_json::Value {
        serde_json::json!({
            "tag_name": "v0.2.2",
            "assets": [
                {
                    "name": "bursar-linux-x86_64",
                    "browser_download_url": "https://github.com/trout-dev-fwd/bursar/releases/download/v0.2.2/bursar-linux-x86_64"
                },
                {
                    "name": "bursar-windows-x86_64.exe",
                    "browser_download_url": "https://github.com/trout-dev-fwd/bursar/releases/download/v0.2.2/bursar-windows-x86_64.exe"
                },
                {
                    "name": "checksums.txt",
                    "browser_download_url": "https://github.com/trout-dev-fwd/bursar/releases/download/v0.2.2/checksums.txt"
                }
            ]
        })
    }

    #[test]
    fn parse_available_update() {
        let json = sample_api_response();
        let result = parse_update_check(&json, "0.2.0", "bursar-linux-x86_64");
        match result {
            UpdateCheck::Available {
                version,
                asset_url,
                checksum_url,
                asset_name,
            } => {
                assert_eq!(version, "0.2.2");
                assert!(asset_url.contains("bursar-linux-x86_64"));
                assert!(checksum_url.contains("checksums.txt"));
                assert_eq!(asset_name, "bursar-linux-x86_64");
            }
            _ => panic!("expected Available"),
        }
    }

    #[test]
    fn parse_up_to_date_same_version() {
        let json = sample_api_response();
        let result = parse_update_check(&json, "0.2.2", "bursar-linux-x86_64");
        assert!(matches!(result, UpdateCheck::UpToDate));
    }

    #[test]
    fn parse_up_to_date_newer_local() {
        let json = sample_api_response();
        let result = parse_update_check(&json, "0.3.0", "bursar-linux-x86_64");
        assert!(matches!(result, UpdateCheck::UpToDate));
    }

    #[test]
    fn version_comparison_older_remote() {
        let json = serde_json::json!({
            "tag_name": "v0.2.0",
            "assets": []
        });
        // 0.2.0 remote <= 0.2.1 local → UpToDate
        let result = parse_update_check(&json, "0.2.1", "bursar-linux-x86_64");
        assert!(matches!(result, UpdateCheck::UpToDate));
    }

    #[test]
    fn parse_missing_platform_asset() {
        let json = sample_api_response();
        // Request an asset name not in the response.
        let result = parse_update_check(&json, "0.2.0", "bursar-macos-aarch64");
        match result {
            UpdateCheck::Failed(msg) => assert!(msg.contains("bursar-macos-aarch64")),
            _ => panic!("expected Failed"),
        }
    }

    #[test]
    fn parse_missing_checksums_txt() {
        let json = serde_json::json!({
            "tag_name": "v0.2.2",
            "assets": [
                {
                    "name": "bursar-linux-x86_64",
                    "browser_download_url": "https://example.com/linux"
                }
            ]
        });
        let result = parse_update_check(&json, "0.2.0", "bursar-linux-x86_64");
        match result {
            UpdateCheck::Failed(msg) => assert!(msg.contains("checksums.txt")),
            _ => panic!("expected Failed"),
        }
    }
}
