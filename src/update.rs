//! GitHub release update check.

use std::time::Duration;

use anyhow::Result;

/// Returns `Ok(Some("1.2.0"))` if a newer version exists, `Ok(None)` if up to date,
/// or `Err` on any failure (caller should silently ignore errors).
pub fn check_for_update(github_repo: &str, current_version: &str) -> Result<Option<String>> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        github_repo
    );
    let response = ureq::get(&url)
        .set("User-Agent", "bursar-update-check") // GitHub API requires User-Agent
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

/// Returns `true` if `remote` is a strictly newer semver than `local`.
/// Returns `false` for equal versions, older remote, or unparseable input.
fn is_newer(remote: &str, local: &str) -> bool {
    let parse = |s: &str| -> Option<Vec<u32>> {
        s.split('.').map(|part| part.parse::<u32>().ok()).collect()
    };
    let (Some(r), Some(l)) = (parse(remote), parse(local)) else {
        return false;
    };
    if r.is_empty() || l.is_empty() {
        return false;
    }
    for (rv, lv) in r.iter().zip(l.iter()) {
        match rv.cmp(lv) {
            std::cmp::Ordering::Greater => return true,
            std::cmp::Ordering::Less => return false,
            std::cmp::Ordering::Equal => continue,
        }
    }
    r.len() > l.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer() {
        assert!(is_newer("1.2.0", "1.1.0"));
        assert!(is_newer("2.0.0", "1.9.9"));
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(!is_newer("1.1.0", "1.1.0")); // same version
        assert!(!is_newer("1.0.0", "1.1.0")); // older
        assert!(!is_newer("1.0.0", "2.0.0")); // older
        assert!(!is_newer("garbage", "1.0.0"));
        assert!(!is_newer("1.0.0", "garbage"));
        assert!(!is_newer("", "1.0.0"));
        assert!(!is_newer("1.0.0", ""));
    }
}
