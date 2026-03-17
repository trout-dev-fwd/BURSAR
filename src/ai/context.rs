use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Converts an entity name to a filesystem-safe slug.
///
/// Rules:
/// - Lowercase all characters
/// - Spaces become underscores
/// - Non-alphanumeric characters (except underscores) are stripped
///
/// # Examples
/// - `"Acme Land LLC"` → `"acme_land_llc"`
/// - `"Bob's Café & Grill"` → `"bobs_caf_grill"`
pub fn slugify_entity_name(name: &str) -> String {
    let raw: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else if c == ' ' {
                '_'
            } else {
                '\0' // sentinel for removal
            }
        })
        .filter(|&c| c != '\0')
        .collect();
    // Collapse consecutive underscores and trim trailing/leading underscores
    let mut result = String::with_capacity(raw.len());
    let mut prev_underscore = false;
    for c in raw.chars() {
        if c == '_' {
            if !prev_underscore {
                result.push('_');
            }
            prev_underscore = true;
        } else {
            result.push(c);
            prev_underscore = false;
        }
    }
    result.trim_matches('_').to_string()
}

/// Returns the path to the context markdown file for an entity.
///
/// The file is named `{slugified_entity_name}.md` inside `context_dir`.
pub fn context_file_path(entity_name: &str, context_dir: &str) -> PathBuf {
    let slug = slugify_entity_name(entity_name);
    Path::new(context_dir).join(format!("{slug}.md"))
}

/// Skeleton content written when a context file is auto-created.
fn skeleton_content(entity_name: &str) -> String {
    format!(
        "# {entity_name} — AI Context\n\n\
         ## Business Context\n\
         <!-- Describe your business here for better AI assistance -->\n"
    )
}

/// Reads the entity context file, auto-creating it with a skeleton if it does not exist.
///
/// Also creates `context_dir` if it does not exist.
pub fn read_context(entity_name: &str, context_dir: &str) -> Result<String> {
    let dir = Path::new(context_dir);
    std::fs::create_dir_all(dir)
        .with_context(|| format!("Failed to create context directory: {}", dir.display()))?;

    let path = context_file_path(entity_name, context_dir);
    if path.exists() {
        return std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read context file: {}", path.display()));
    }

    // Auto-create with skeleton
    let skeleton = skeleton_content(entity_name);
    std::fs::write(&path, &skeleton)
        .with_context(|| format!("Failed to create context file: {}", path.display()))?;
    Ok(skeleton)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn slugify_basic_business_name() {
        assert_eq!(slugify_entity_name("Acme Land LLC"), "acme_land_llc");
    }

    #[test]
    fn slugify_strips_apostrophe_and_ampersand() {
        // "Bob's Café & Grill" — apostrophe, accent, and & are stripped
        // 'é' is non-ASCII alphanumeric — behavior: strip (only ASCII alphanumeric kept)
        let result = slugify_entity_name("Bob's Café & Grill");
        assert_eq!(result, "bobs_caf_grill");
    }

    #[test]
    fn slugify_all_lowercase() {
        assert_eq!(slugify_entity_name("UPPERCASE LLC"), "uppercase_llc");
    }

    #[test]
    fn slugify_numbers_preserved() {
        assert_eq!(
            slugify_entity_name("Entity 42 Holdings"),
            "entity_42_holdings"
        );
    }

    #[test]
    fn slugify_empty_string() {
        assert_eq!(slugify_entity_name(""), "");
    }

    #[test]
    fn slugify_only_special_chars() {
        assert_eq!(slugify_entity_name("&&&"), "");
    }

    #[test]
    fn context_file_path_correct() {
        let path = context_file_path("Acme Land LLC", "/tmp/context");
        assert_eq!(path, PathBuf::from("/tmp/context/acme_land_llc.md"));
    }

    #[test]
    fn read_context_auto_creates_file_and_directory() {
        let dir = std::env::temp_dir()
            .join("accounting_test_context_autocreate")
            .join("subdir");
        let dir_str = dir.to_string_lossy().to_string();

        // Ensure the directory does not exist
        let _ = fs::remove_dir_all(&dir);

        let content = read_context("Acme Land LLC", &dir_str).expect("read_context should succeed");
        assert!(content.contains("Acme Land LLC — AI Context"));
        assert!(content.contains("Business Context"));

        // File should now exist
        let path = context_file_path("Acme Land LLC", &dir_str);
        assert!(path.exists(), "context file should have been created");

        // Cleanup
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_context_returns_skeleton_contents() {
        let dir = std::env::temp_dir().join("accounting_test_context_skeleton");
        let dir_str = dir.to_string_lossy().to_string();
        let _ = fs::remove_dir_all(&dir);

        let content = read_context("Test Entity", &dir_str).expect("read_context");
        let skeleton = skeleton_content("Test Entity");
        assert_eq!(content, skeleton);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_context_returns_existing_file_unchanged() {
        let dir = std::env::temp_dir().join("accounting_test_context_existing");
        let dir_str = dir.to_string_lossy().to_string();
        fs::create_dir_all(&dir).unwrap();

        let path = context_file_path("My Entity", &dir_str);
        let custom = "# My Entity\n\nCustom content here.\n";
        fs::write(&path, custom).unwrap();

        let content = read_context("My Entity", &dir_str).expect("read_context");
        assert_eq!(content, custom);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_context_auto_create_contains_entity_name_in_heading() {
        let dir = std::env::temp_dir().join("accounting_test_context_heading");
        let dir_str = dir.to_string_lossy().to_string();
        let _ = fs::remove_dir_all(&dir);

        let content = read_context("Sunrise Farms Inc", &dir_str).expect("read_context");
        assert!(
            content.contains("Sunrise Farms Inc"),
            "skeleton should include entity name: {}",
            content
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
