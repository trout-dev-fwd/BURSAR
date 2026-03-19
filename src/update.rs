//! GitHub release update check, download, verification, and binary replacement.

use std::path::{Path, PathBuf};
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

// ── Pre-flight checks ─────────────────────────────────────────────────────────

/// Run pre-flight checks to verify the binary can be replaced.
///
/// Returns `Ok(exe_path)` if checks pass, `Err(reason)` if not.
///
/// Checks performed:
/// 1. Resolves the running binary's path via `current_exe()`.
/// 2. Symlink detection: compares the resolved path against `argv[0]` canonicalized.
///    If they differ (or argv[0] is itself a symlink), the binary is accessed via a
///    symlink. Renaming at the resolved path would leave the symlink pointing at `.old`.
/// 3. Write permission: creates and immediately deletes a temp file in the binary's
///    parent directory. If creation fails, the directory is not writable.
pub fn preflight_check() -> Result<PathBuf, String> {
    let exe_path =
        std::env::current_exe().map_err(|e| format!("failed to resolve binary path: {e}"))?;

    // Symlink detection: if argv[0] canonicalises to a different path than current_exe(),
    // the binary was launched through a symlink. current_exe() resolves through /proc/self/exe
    // on Linux (already resolved), so we check the argv[0] path directly.
    //
    // Approach: get argv[0], canonicalize it, and compare to current_exe(). If they differ,
    // a symlink is in the path.
    if let Some(argv0) = std::env::args().next() {
        let argv0_path = Path::new(&argv0);
        // If argv0 exists as a path and canonicalises to a different location than
        // current_exe(), the binary was accessed via a symlink. Renaming at the resolved
        // path would leave the symlink pointing at `.old`.
        //
        // If argv0 does not exist as a path (e.g., launched from PATH without `./`),
        // we skip this check to avoid a false positive.
        if argv0_path.exists()
            && let Ok(canonical_argv0) = std::fs::canonicalize(argv0_path)
            && canonical_argv0 != exe_path
        {
            return Err("binary is a symlink. Replace it manually.".to_string());
        }
    }

    // Write permission check: try creating a temp file in the binary's parent directory.
    let parent_dir = exe_path
        .parent()
        .ok_or_else(|| "binary has no parent directory".to_string())?;

    let temp_path = parent_dir.join(format!(".bursar-update-check-{}", std::process::id()));
    std::fs::File::create(&temp_path).map_err(|_| {
        format!(
            "permission denied on {}. Try running with appropriate permissions or \
             moving the binary to a user-writable location.",
            parent_dir.display()
        )
    })?;
    // Clean up immediately — we only needed to test write access.
    let _ = std::fs::remove_file(&temp_path);

    Ok(exe_path)
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

    // ── preflight_check tests ─────────────────────────────────────────────────

    #[test]
    fn preflight_writable_temp_dir_passes() {
        // We can't call preflight_check() directly (it uses current_exe()),
        // but we can test the write-permission sub-check by attempting a temp
        // file in a known-writable directory.
        let dir = std::env::temp_dir();
        let temp_path = dir.join(".bursar-update-check-test-write");
        assert!(
            std::fs::File::create(&temp_path).is_ok(),
            "should be able to write to temp dir"
        );
        let _ = std::fs::remove_file(&temp_path);
    }

    #[test]
    #[cfg(unix)]
    fn preflight_read_only_dir_fails() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join("bursar_update_ro_test");
        std::fs::create_dir_all(&dir).expect("create dir failed");

        // Make directory read-only.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555))
            .expect("set permissions failed");

        let temp_path = dir.join(".bursar-update-check-test");
        let result = std::fs::File::create(&temp_path);

        // Restore permissions before any assertions so cleanup can succeed.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))
            .expect("restore permissions failed");
        let _ = std::fs::remove_dir(&dir);

        assert!(result.is_err(), "write to read-only dir should fail");
    }
}
