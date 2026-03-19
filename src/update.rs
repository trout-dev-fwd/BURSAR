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

// ── Download with progress ─────────────────────────────────────────────────────

/// Downloads a binary from `url` to `dest`, calling `on_progress(bytes_received, total)`
/// after each chunk. `total` is `None` when `Content-Length` is absent.
///
/// Deletes any partial file on error. Uses a 300-second timeout.
pub fn download_with_progress<F>(url: &str, dest: &Path, on_progress: F) -> Result<(), String>
where
    F: FnMut(u64, Option<u64>),
{
    let user_agent = format!("bursar/{}", env!("CARGO_PKG_VERSION"));
    let response = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(300))
        .build()
        .get(url)
        .set("User-Agent", &user_agent)
        .set("Accept", "application/octet-stream")
        .call()
        .map_err(|e| format!("download request failed: {e}"))?;

    let content_length: Option<u64> = response
        .header("Content-Length")
        .and_then(|v| v.parse().ok());

    read_chunks_to_file(response.into_reader(), dest, content_length, on_progress)
}

/// Reads `reader` in 8 KB chunks, writing to `dest` and calling `on_progress` after each.
/// Deletes `dest` on any I/O error. Verifies size against `content_length` if provided.
///
/// Separated from `download_with_progress` so the chunking + progress logic is testable
/// without a live HTTP connection.
fn read_chunks_to_file<R, F>(
    mut reader: R,
    dest: &Path,
    content_length: Option<u64>,
    mut on_progress: F,
) -> Result<(), String>
where
    R: std::io::Read,
    F: FnMut(u64, Option<u64>),
{
    let mut file = std::fs::File::create(dest)
        .map_err(|e| format!("failed to create destination file: {e}"))?;

    let mut buf = [0u8; 8192];
    let mut bytes_received: u64 = 0;

    loop {
        let n = reader.read(&mut buf).map_err(|e| {
            // Partial file cleanup on read error.
            let _ = std::fs::remove_file(dest);
            format!("download read error: {e}")
        })?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buf[..n]).map_err(|e| {
            let _ = std::fs::remove_file(dest);
            format!("download write error: {e}")
        })?;
        bytes_received += n as u64;
        on_progress(bytes_received, content_length);
    }

    // Verify final size matches Content-Length if provided.
    if let Some(expected) = content_length
        && bytes_received != expected
    {
        let _ = std::fs::remove_file(dest);
        return Err(format!(
            "download size mismatch: expected {expected} bytes, got {bytes_received}"
        ));
    }

    Ok(())
}

// ── SHA256 verification ───────────────────────────────────────────────────────

/// Downloads `checksum_url` (a `sha256sum`-format text file) and extracts the expected
/// hex hash for `asset_name`. Returns `Err` if the file can't be fetched or the asset
/// is not listed.
pub fn fetch_expected_checksum(checksum_url: &str, asset_name: &str) -> Result<String, String> {
    let user_agent = format!("bursar/{}", env!("CARGO_PKG_VERSION"));
    let response = ureq::get(checksum_url)
        .set("User-Agent", &user_agent)
        .set("Accept", "application/octet-stream")
        .timeout(Duration::from_secs(30))
        .call()
        .map_err(|e| format!("failed to fetch checksums: {e}"))?;

    let body = response
        .into_string()
        .map_err(|e| format!("failed to read checksum body: {e}"))?;

    parse_checksum_file(&body, asset_name)
}

/// Parses a `sha256sum`-format checksum file and returns the hex hash for `asset_name`.
/// Expected format: `{hex_hash}  {filename}` (two spaces, matching `sha256sum` output).
fn parse_checksum_file(body: &str, asset_name: &str) -> Result<String, String> {
    body.lines()
        .find_map(|line| {
            // Format: "{hash}  {filename}" — split at the two-space separator.
            let (hash, name) = line.split_once("  ")?;
            if name.trim() == asset_name {
                Some(hash.trim().to_string())
            } else {
                None
            }
        })
        .ok_or_else(|| format!("'{asset_name}' not found in checksums file"))
}

/// Computes the SHA256 of `file_path` and compares it to `expected_hex`.
/// Returns `Err` with both hashes on mismatch.
pub fn verify_checksum(file_path: &Path, expected_hex: &str) -> Result<(), String> {
    use sha2::Digest;

    let mut file = std::fs::File::open(file_path)
        .map_err(|e| format!("failed to open file for hashing: {e}"))?;

    let mut hasher = sha2::Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = std::io::Read::read(&mut file, &mut buf)
            .map_err(|e| format!("read error during checksum: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    let actual_hex: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    if actual_hex.eq_ignore_ascii_case(expected_hex) {
        Ok(())
    } else {
        Err(format!(
            "checksum mismatch — expected: {expected_hex}, got: {actual_hex}"
        ))
    }
}

// ── Binary replacement and restart ────────────────────────────────────────────

/// Replaces the running binary with `new_binary` and restarts the process.
///
/// **Does not return on success.** On failure, attempts rollback (restoring the old
/// binary) and returns an error string.
///
/// Rename sequence:
/// 1. `current_exe` → `current_exe.old`  (rollback point)
/// 2. `new_binary`  → `current_exe`      (atomic swap)
/// 3. Linux: `exec()` replaces the current process with the new binary.
///    Windows: restores terminal, spawns new binary, then exits.
pub fn replace_and_restart(
    current_exe: &Path,
    new_binary: &Path,
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
) -> Result<(), String> {
    let old_path = make_old_path(current_exe);

    // Step 1: rename current → .old (rollback point; nothing changes if this fails).
    std::fs::rename(current_exe, &old_path)
        .map_err(|e| format!("failed to rename current binary to .old: {e}"))?;

    // Step 2: rename new → current.
    if let Err(e) = std::fs::rename(new_binary, current_exe) {
        // Attempt rollback: restore old binary.
        let _ = std::fs::rename(&old_path, current_exe);
        return Err(format!("failed to rename new binary into place: {e}"));
    }

    // Step 3: platform-specific restart.
    do_restart(current_exe, terminal)
}

/// Constructs the `.old` path: same directory, filename with `.old` appended.
fn make_old_path(exe: &Path) -> PathBuf {
    let filename = exe
        .file_name()
        .map(|n| {
            let mut s = n.to_os_string();
            s.push(".old");
            s
        })
        .unwrap_or_else(|| std::ffi::OsString::from("bursar.old"));
    exe.parent().unwrap_or(Path::new(".")).join(filename)
}

#[cfg(target_os = "linux")]
fn do_restart(
    current_exe: &Path,
    _terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;

    // Set executable permission on the new binary.
    std::fs::set_permissions(current_exe, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("failed to set executable permissions: {e}"))?;

    // exec() replaces the current process image. On success it never returns.
    // We pass the same args that were used to start the current process.
    let err = std::process::Command::new(current_exe)
        .args(std::env::args_os().skip(1))
        .exec();

    // exec() only returns if it failed.
    Err(format!("exec failed: {err}"))
}

#[cfg(target_os = "windows")]
fn do_restart(
    current_exe: &Path,
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
) -> Result<(), String> {
    // On Windows, restore the terminal before spawning so we don't leave it in raw mode.
    drop(terminal);
    let _ = crossterm::terminal::disable_raw_mode();
    let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen);

    std::process::Command::new(current_exe)
        .args(std::env::args().skip(1))
        .spawn()
        .map_err(|e| format!("failed to spawn new binary: {e}"))?;

    std::process::exit(0);
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn do_restart(
    _current_exe: &Path,
    _terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
) -> Result<(), String> {
    Err("restart not supported on this platform".to_string())
}

/// Cleans up the `.old` binary left by a previous update. Silent on all failures.
pub fn cleanup_old_binary() {
    if let Ok(exe) = std::env::current_exe() {
        let old_path = make_old_path(&exe);
        let _ = std::fs::remove_file(old_path);
    }
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

    // ── download progress tests ───────────────────────────────────────────────

    #[test]
    fn progress_callback_receives_incrementing_counts() {
        let data = b"hello world this is test data for progress tracking";
        let reader = std::io::Cursor::new(data);
        let dest = std::env::temp_dir().join("bursar_dl_progress_test.bin");

        let mut calls: Vec<(u64, Option<u64>)> = Vec::new();
        read_chunks_to_file(reader, &dest, Some(data.len() as u64), |recv, total| {
            calls.push((recv, total));
        })
        .expect("should succeed");

        let _ = std::fs::remove_file(&dest);

        // Data fits in one 8 KB chunk, so one progress call.
        assert!(!calls.is_empty(), "expected at least one progress call");
        let (final_recv, final_total) = calls.last().copied().unwrap();
        assert_eq!(final_recv, data.len() as u64);
        assert_eq!(final_total, Some(data.len() as u64));

        // Each call should have non-decreasing byte count.
        let mut prev = 0u64;
        for (recv, _) in &calls {
            assert!(*recv >= prev, "byte count must be non-decreasing");
            prev = *recv;
        }
    }

    #[test]
    fn partial_file_cleaned_up_on_size_mismatch() {
        let data = b"short";
        let reader = std::io::Cursor::new(data);
        let dest = std::env::temp_dir().join("bursar_dl_mismatch_test.bin");

        // Claim content-length is 100 bytes but only 5 bytes are provided.
        let result = read_chunks_to_file(reader, &dest, Some(100), |_, _| {});

        assert!(result.is_err(), "expected size mismatch error");
        assert!(
            !dest.exists(),
            "partial file should be cleaned up on size mismatch"
        );
    }

    // ── SHA256 verification tests ─────────────────────────────────────────────

    #[test]
    fn parse_checksum_file_finds_correct_hash() {
        let body = "\
a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2  bursar-linux-x86_64\n\
e5f6g7h8e5f6g7h8e5f6g7h8e5f6g7h8e5f6g7h8e5f6g7h8e5f6g7h8e5f6g7h8  bursar-windows-x86_64.exe\n";
        let result = parse_checksum_file(body, "bursar-linux-x86_64");
        assert_eq!(
            result.unwrap(),
            "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"
        );
    }

    #[test]
    fn parse_checksum_file_asset_not_found() {
        let body = "a1b2c3d4  bursar-linux-x86_64\n";
        let result = parse_checksum_file(body, "bursar-macos-aarch64");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("bursar-macos-aarch64"));
    }

    #[test]
    fn verify_checksum_correct_hash() {
        // Write known bytes to a temp file and verify the known SHA256.
        let dest = std::env::temp_dir().join("bursar_sha256_test.bin");
        std::fs::write(&dest, b"hello world").expect("write failed");

        // SHA256 of b"hello world" (no newline).
        let expected = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        // Attempt verification — if the hash constant is wrong the test fails here
        // and we update the constant. The real SHA256 value is computed by sha2.
        let result = verify_checksum(&dest, expected);
        let _ = std::fs::remove_file(&dest);

        // Accept either Ok (hash correct) or a mismatch error that tells us the real hash.
        // In CI, we verify the hash is stable by always expecting Ok.
        assert!(
            result.is_ok(),
            "checksum mismatch: {result:?}\n\
             Update the expected hash in the test if sha2 produces a different value."
        );
    }

    #[test]
    fn verify_checksum_wrong_hash_produces_error() {
        let dest = std::env::temp_dir().join("bursar_sha256_wrong_test.bin");
        std::fs::write(&dest, b"hello world").expect("write failed");

        let result = verify_checksum(
            &dest,
            "0000000000000000000000000000000000000000000000000000000000000000",
        );
        let _ = std::fs::remove_file(&dest);

        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("expected:"),
            "error should include expected hash: {msg}"
        );
        assert!(
            msg.contains("got:"),
            "error should include actual hash: {msg}"
        );
    }

    // ── binary replacement tests ──────────────────────────────────────────────

    #[test]
    fn cleanup_old_binary_no_panic_when_file_absent() {
        // Should silently succeed even when there is no .old file to delete.
        cleanup_old_binary(); // Must not panic.
    }

    #[test]
    fn make_old_path_appends_old_suffix() {
        let exe = Path::new("/home/user/bin/bursar");
        let old = make_old_path(exe);
        assert_eq!(old, Path::new("/home/user/bin/bursar.old"));
    }

    #[test]
    fn rename_sequence_moves_files_correctly() {
        let dir = std::env::temp_dir().join("bursar_rename_test");
        std::fs::create_dir_all(&dir).expect("create dir");

        let current = dir.join("bursar");
        let new_bin = dir.join("bursar.new");
        std::fs::write(&current, b"old binary").expect("write current");
        std::fs::write(&new_bin, b"new binary").expect("write new");

        let old_path = make_old_path(&current);

        // Simulate the rename steps from replace_and_restart (without exec).
        std::fs::rename(&current, &old_path).expect("rename current -> old");
        std::fs::rename(&new_bin, &current).expect("rename new -> current");

        assert_eq!(std::fs::read(&current).unwrap(), b"new binary");
        assert_eq!(std::fs::read(&old_path).unwrap(), b"old binary");
        assert!(!new_bin.exists());

        // Cleanup.
        let _ = std::fs::remove_file(&current);
        let _ = std::fs::remove_file(&old_path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn rename_sequence_rolls_back_on_step2_failure() {
        let dir = std::env::temp_dir().join("bursar_rollback_test");
        std::fs::create_dir_all(&dir).expect("create dir");

        let current = dir.join("bursar");
        std::fs::write(&current, b"old binary").expect("write current");

        let old_path = make_old_path(&current);

        // Step 1: rename current -> old.
        std::fs::rename(&current, &old_path).expect("rename current -> old");

        // Step 2 fails: new binary does not exist.
        let new_bin = dir.join("bursar.new.nonexistent");
        let rename_result = std::fs::rename(&new_bin, &current);

        // Simulate rollback on failure.
        if rename_result.is_err() {
            let _ = std::fs::rename(&old_path, &current);
        }

        // After rollback, the original binary is restored.
        assert!(current.exists(), "original binary should be restored");
        assert!(!old_path.exists(), "old file should be gone after rollback");

        let _ = std::fs::remove_file(&current);
        let _ = std::fs::remove_dir(&dir);
    }
}
