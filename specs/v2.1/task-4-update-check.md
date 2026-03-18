# Task 4: Update Check via GitHub API

Read `specs/handoff.md` for full codebase orientation, then read `specs/v2.1-build-spec.md` for the complete V2.1 plan. This prompt covers Task 4 only. Tasks 1-3 have been completed — the crate is "bursar", the startup screen exists with entity management, and the splash phase currently just sleeps for 1 second.

## What to Do

Replace the 1-second sleep in the splash phase with a GitHub release check that fetches the latest version, then passes the result to the startup screen for display.

## Part A: `check_for_update` Function

Create this in a sensible location — either a new `src/update.rs` or inside the startup screen module.

```rust
use anyhow::Result;
use std::time::Duration;

/// Returns `Ok(Some("1.2.0"))` if a newer version exists, `Ok(None)` if up to date,
/// or `Err` on any failure (caller should silently ignore errors).
pub fn check_for_update(github_repo: &str, current_version: &str) -> Result<Option<String>> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        github_repo
    );
    let response = ureq::get(&url)
        .set("User-Agent", "bursar-update-check")  // GitHub API requires User-Agent
        .timeout(Duration::from_secs(3))
        .call()?;

    let json: serde_json::Value = response.into_json()?;
    let tag = json["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing tag_name in response"))?;
    let remote = tag.strip_prefix('v').unwrap_or(tag);

    if is_newer(remote, current_version) {
        Ok(Some(remote.to_string()))
    } else {
        Ok(None)
    }
}
```

## Part B: Version Comparison

```rust
/// Returns true if `remote` is a newer semver than `local`.
/// Returns false if either string fails to parse.
fn is_newer(remote: &str, local: &str) -> bool {
    let parse = |s: &str| -> Option<Vec<u32>> {
        s.split('.').map(|part| part.parse::<u32>().ok()).collect()
    };
    let (Some(r), Some(l)) = (parse(remote), parse(local)) else {
        return false;
    };
    for (rv, lv) in r.iter().zip(l.iter()) {
        match rv.cmp(lv) {
            std::cmp::Ordering::Greater => return true,
            std::cmp::Ordering::Less => return false,
            std::cmp::Ordering::Equal => continue,
        }
    }
    // If all compared segments are equal, remote is newer only if it has more segments
    // e.g. "1.0.0.1" > "1.0.0" — though this is unusual
    r.len() > l.len()
}
```

### Unit Tests for `is_newer`

Write these as `#[cfg(test)]` tests:

```rust
#[test]
fn test_is_newer() {
    assert!(is_newer("1.2.0", "1.1.0"));
    assert!(is_newer("2.0.0", "1.9.9"));
    assert!(is_newer("1.0.1", "1.0.0"));
    assert!(is_newer("0.2.0", "0.1.9"));
    assert!(!is_newer("1.1.0", "1.1.0"));  // same version
    assert!(!is_newer("1.0.0", "1.1.0"));  // older
    assert!(!is_newer("1.0.0", "2.0.0"));  // older
    assert!(!is_newer("garbage", "1.0.0"));
    assert!(!is_newer("1.0.0", "garbage"));
    assert!(!is_newer("", "1.0.0"));
    assert!(!is_newer("1.0.0", ""));
}
```

## Part C: workspace.toml Config

Support an optional `[updates]` section:

```toml
[updates]
github_repo = "owner/bursar"
```

Read this from the workspace config. If the `[updates]` section is missing or `github_repo` is absent, skip the update check entirely — no error, no log.

This likely requires adding an `updates` field to the workspace config struct in `src/config.rs`. Check how the existing config parsing works and extend it.

## Part D: Modify the Splash Phase

In the wrapper loop in `main.rs`, replace the current `sleep(1s)` with:

```
Splash =>
    let start = Instant::now();

    // Render splash with logo + version
    terminal.draw(|f| render_splash(f, version, ""))?;

    // Check for updates if configured
    let update_notice = if let Some(repo) = config.updates_github_repo() {
        // Re-render with "Checking for updates..."
        terminal.draw(|f| render_splash(f, version, "Checking for updates..."))?;

        match check_for_update(&repo, env!("CARGO_PKG_VERSION")) {
            Ok(Some(new_ver)) => Some(format!(
                "New version v{} available — github.com/{}/releases",
                new_ver, repo
            )),
            _ => None,  // up to date, or error — either way, no notice
        }
    } else {
        None
    };

    // Ensure at least 1 second of splash
    let elapsed = start.elapsed();
    if elapsed < Duration::from_secs(1) {
        std::thread::sleep(Duration::from_secs(1) - elapsed);
    }

    state = Startup(StartupScreen::new(config, update_notice))
```

The `render_splash` function renders the ASCII banner centered, version right-aligned below it, and an optional status message ("Checking for updates...") below that.

## Part E: Display Update Notice on Startup Screen

The `StartupScreen` already accepts `update_notice: Option<String>` (from Task 2). Now render it:

- Position: below the version number, above the entity list
- Style: `Color::Yellow` foreground to stand out
- Content: the formatted string from Part D, e.g. "New version v1.2.0 available — github.com/owner/bursar/releases"
- If `update_notice` is `None`, this area is blank (no vertical space consumed)

## Verification Checklist

```bash
cargo fmt
cargo clippy -D warnings
cargo test
```

Then manually verify these scenarios:

1. **With valid `[updates]` config + internet**: Splash shows "Checking for updates...", then startup screen shows yellow notice if a newer version exists
2. **With valid config + no internet / timeout**: Splash shows "Checking for updates..." for ~3 seconds, then transitions to startup screen with no notice and no error
3. **With no `[updates]` section**: Splash shows for 1 second, no check attempted, no "Checking for updates..." text
4. **Version comparison**: Unit tests all pass
5. **Splash always shows for at least 1 second** regardless of how fast the check completes

## Commit

```
V2.1, Task 4: Add GitHub update check on startup
```
