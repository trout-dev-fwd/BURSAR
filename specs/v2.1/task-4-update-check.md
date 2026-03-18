# Task 4: Update Check via GitHub API + Config Parsing Bug Fix

Read `specs/handoff.md` for full codebase orientation, then read `specs/v2.1-build-spec.md` for the complete V2.1 plan. This prompt covers Task 4 only. Tasks 1-3 (plus fixes 2b and 3b) have been completed — the crate is "bursar", the startup screen exists with entity management, and the splash phase currently just sleeps for 1 second.

## PRIORITY BUG FIX: Config Parsing with Missing Sections

### The Problem

When all entities are deleted from the startup screen, workspace.toml ends up with no `[[entities]]` block and no `[ai]` section. The `reload_entities` call then fails with "Failed to parse config file" because `WorkspaceConfig` in `src/config.rs` has required fields that aren't present in the file.

Example of a valid-but-minimal workspace.toml that currently fails to parse:

```toml
report_output_dir = "~/bursar/reports"
last_opened_entity = "trout-home"
```

### The Fix

In `src/config.rs`, ensure ALL fields on `WorkspaceConfig` (and any nested config structs like the AI config) have `#[serde(default)]` so that missing sections deserialize to sensible defaults:

- `entities` → defaults to empty `Vec`
- `ai` section → defaults to whatever makes sense (empty persona, default model, etc.)
- `context_dir` → defaults to `None` or `"context"`
- Any other field that could be absent

After fixing, verify that this minimal workspace.toml parses without error:

```toml
report_output_dir = "~/bursar/reports"
```

And this completely empty file also parses:

```toml
```

The startup screen already handles an empty entity list gracefully (shows "Press 'a' to add one") — the bug is that the config parser rejects the file before the startup screen ever sees it.

### Test to Add

```rust
#[test]
fn test_parse_minimal_config() {
    // A workspace.toml with no entities and no ai section should parse successfully
    let toml_str = r#"report_output_dir = "~/bursar/reports""#;
    let config: WorkspaceConfig = toml::from_str(toml_str).expect("should parse minimal config");
    assert!(config.entities.is_empty());
}

#[test]
fn test_parse_empty_config() {
    let toml_str = "";
    let config: WorkspaceConfig = toml::from_str(toml_str).expect("should parse empty config");
    assert!(config.entities.is_empty());
}
```

---

## FEATURE: Update Check via GitHub API

### workspace.toml addition

Support an optional `[updates]` section:

```toml
[updates]
github_repo = "owner/bursar"
```

If `[updates]` or `github_repo` is absent, skip the update check entirely — no error, no log. This section must also be optional in the config parser (same `#[serde(default)]` treatment as the bug fix above).

### `check_for_update` function

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

### Version comparison: `is_newer(remote, local) -> bool`

```rust
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
    r.len() > l.len()
}
```

### Unit tests for `is_newer`

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

### Modify the splash phase

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

### Display update notice on startup screen

The `StartupScreen` already has an `update_notice: Option<String>` field. If it's currently hardcoded to `None`, change the constructor to accept it as a parameter. Then render it:

- Position: below the version number, above the entity list
- Style: `Color::Yellow` foreground to stand out
- Content: the formatted string from above, e.g. "New version v1.2.0 available — github.com/owner/bursar/releases"
- If `None`, this area is blank (no vertical space consumed)

### Config integration

Add an `[updates]` section to the workspace config struct in `src/config.rs`:

```rust
#[serde(default)]
pub updates: UpdatesConfig,

// ...

#[derive(Debug, Deserialize, Default)]
pub struct UpdatesConfig {
    pub github_repo: Option<String>,
}
```

Add a convenience method:

```rust
impl WorkspaceConfig {
    pub fn updates_github_repo(&self) -> Option<&str> {
        self.updates.github_repo.as_deref()
    }
}
```

## Verification

```bash
cargo fmt
cargo clippy -D warnings
cargo test
```

Then manually verify all scenarios:

**Config parsing bug fix:**
- Delete all entities from the startup screen — no parse error, shows empty list with "Press 'a' to add"
- Manually create a workspace.toml with only `report_output_dir` — app starts, shows empty entity list
- Empty workspace.toml — app starts with defaults

**Update check:**
- With valid `[updates]` config and internet: splash shows "Checking for updates...", startup screen shows yellow notice if remote version is newer
- With valid config but no internet / timeout: splash shows "Checking for updates..." for ~3 seconds (timeout), transitions to startup screen normally with no error
- With no `[updates]` section: splash shows for 1 second, no check attempted, no "Checking for updates..." text
- Version comparison unit tests pass
- The splash screen renders the logo and "Checking for updates..." before the blocking call

## Commit

```
V2.1, Task 4: Fix config parsing for empty entities, add GitHub update check
```
