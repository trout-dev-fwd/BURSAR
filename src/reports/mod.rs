//! Report trait, shared formatting utilities, and file output for all 8 reports.
//!
//! All reports implement the `Report` trait and use the shared formatting helpers
//! (`format_header`, `format_table`, `format_money`) to produce consistent `.txt` output
//! with box-drawing characters.

use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::NaiveDate;

use crate::db::EntityDb;
use crate::types::{AccountId, Money};

// ── Sub-modules (filled in Tasks 2-9) ────────────────────────────────────────
pub mod account_detail;
pub mod ap_aging;
pub mod ar_aging;
pub mod balance_sheet;
pub mod cash_flow;
pub mod envelope_budget;
pub mod fixed_asset_schedule;
pub mod income_statement;
pub mod tax_summary;
pub mod trial_balance;

// ── Report trait ─────────────────────────────────────────────────────────────

/// Parameters controlling report generation.
pub struct ReportParams {
    /// Display name of the entity for the report header.
    pub entity_name: String,
    /// Used by point-in-time reports (Balance Sheet, Trial Balance).
    pub as_of_date: Option<NaiveDate>,
    /// Used by period reports (Income Statement, Cash Flow, AR/AP Aging).
    pub date_range: Option<(NaiveDate, NaiveDate)>,
    /// Used by Account Detail report.
    pub account_id: Option<AccountId>,
}

/// Implemented by each of the 8 reports.
pub trait Report {
    /// Human-readable report name (used in the filename and header).
    fn name(&self) -> &str;

    /// Generate the report content. Returns the full formatted text.
    fn generate(&self, db: &EntityDb, params: &ReportParams) -> Result<String>;
}

// ── Box-drawing character constants ──────────────────────────────────────────

/// Horizontal line segment (─).
pub const BOX_H: char = '─';
/// Vertical line segment (│).
pub const BOX_V: char = '│';
/// Top-left corner (┌).
pub const BOX_TL: char = '┌';
/// Top-right corner (┐).
pub const BOX_TR: char = '┐';
/// Bottom-left corner (└).
pub const BOX_BL: char = '└';
/// Bottom-right corner (┘).
pub const BOX_BR: char = '┘';
/// Top T-junction / column divider on top border (┬).
pub const BOX_TM: char = '┬';
/// Bottom T-junction (┴).
pub const BOX_BM: char = '┴';
/// Left T-junction / row divider on left border (├).
pub const BOX_ML: char = '├';
/// Right T-junction (┤).
pub const BOX_MR: char = '┤';
/// Cross junction (┼).
pub const BOX_MM: char = '┼';

// ── Column alignment ─────────────────────────────────────────────────────────

/// Column text alignment for [`format_table`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Builds one horizontal border line using box-drawing corners and junctions.
/// Each column segment is `width + 2` long (1 space padding each side).
fn h_border(widths: &[usize], left: char, mid: char, right: char) -> String {
    let h = BOX_H.to_string();
    let segments: Vec<String> = widths.iter().map(|&w| h.repeat(w + 2)).collect();
    format!("{}{}{}", left, segments.join(&mid.to_string()), right)
}

/// Pads/aligns `cell` within a field of `width` characters (with 1-space side padding).
fn align_cell(cell: &str, width: usize, align: Align) -> String {
    match align {
        Align::Left => format!(" {:<width$} ", cell, width = width),
        Align::Right => format!(" {:>width$} ", cell, width = width),
    }
}

/// Builds one table data/header row with vertical separators.
fn table_row(cells: &[&str], widths: &[usize], alignments: &[Align]) -> String {
    let parts: Vec<String> = cells
        .iter()
        .enumerate()
        .map(|(i, cell)| {
            let w = widths.get(i).copied().unwrap_or(cell.len());
            let a = alignments.get(i).copied().unwrap_or(Align::Left);
            align_cell(cell, w, a)
        })
        .collect();
    format!("{}{}{}", BOX_V, parts.join(&BOX_V.to_string()), BOX_V)
}

// ── Public formatting functions ───────────────────────────────────────────────

/// Formats a `Money` amount as a display string (2 decimal places, thousands separators).
///
/// The right-alignment within a table column is handled by [`format_table`]; this function
/// simply returns the canonical string representation produced by `Money::Display`.
pub fn format_money(amount: Money) -> String {
    amount.to_string()
}

/// Generates a box-drawing header block with the entity name, report title, date line,
/// accounting basis, and generation timestamp.
///
/// ```text
/// ┌──────────────────────────────────────────────┐
/// │               Acme Land LLC                  │
/// │               Trial Balance                  │
/// │           As of March 31, 2026               │
/// │              Accrual Basis                   │
/// │       Generated: Mar 31, 2026 4:11 PM        │
/// └──────────────────────────────────────────────┘
/// ```
///
/// All lines in the returned string have the same character width.
/// The minimum inner width is 40 characters; wider content expands it automatically.
pub fn format_header(entity: &str, title: &str, date_info: &str) -> String {
    const MIN_INNER: usize = 40;

    let basis = "Accrual Basis";
    let generated = chrono::Local::now()
        .format("Generated: %b %-d, %Y %-I:%M %p")
        .to_string();

    // Inner width = max content length, at least MIN_INNER, plus 4 (2 spaces each side).
    let content_max = [entity, title, date_info, basis, &generated]
        .iter()
        .map(|s| s.chars().count())
        .max()
        .unwrap_or(0);
    let inner_width = content_max.max(MIN_INNER) + 4;

    let h_line: String = BOX_H.to_string().repeat(inner_width);
    let top = format!("{}{}{}", BOX_TL, h_line, BOX_TR);
    let bottom = format!("{}{}{}", BOX_BL, h_line, BOX_BR);

    let center_line = |s: &str| -> String {
        let len = s.chars().count();
        let total_pad = inner_width.saturating_sub(len);
        let left_pad = total_pad / 2;
        let right_pad = total_pad - left_pad;
        format!(
            "{}{}{}{}{}",
            BOX_V,
            " ".repeat(left_pad),
            s,
            " ".repeat(right_pad),
            BOX_V
        )
    };

    [
        top,
        center_line(entity),
        center_line(title),
        center_line(date_info),
        center_line(basis),
        center_line(&generated),
        bottom,
    ]
    .join("\n")
}

/// Generates a box-drawing table with auto-calculated column widths.
///
/// Column widths are the maximum of the header label width and the widest data cell in that
/// column. Alignment is specified per-column via the `alignments` slice; columns beyond the
/// slice length default to `Align::Left`.
///
/// ```text
/// ┌──────────────────┬────────────┬────────────┐
/// │ Account          │      Debit │     Credit │
/// ├──────────────────┼────────────┼────────────┤
/// │ Cash             │    500.00  │       0.00 │
/// └──────────────────┴────────────┴────────────┘
/// ```
pub fn format_table(headers: &[&str], rows: &[Vec<String>], alignments: &[Align]) -> String {
    let ncols = headers.len();

    // Auto-calculate column widths from header labels and all data cells.
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < ncols {
                let cell_len = cell.chars().count();
                if cell_len > widths[i] {
                    widths[i] = cell_len;
                }
            }
        }
    }

    let mut lines: Vec<String> = Vec::new();

    // Top border.
    lines.push(h_border(&widths, BOX_TL, BOX_TM, BOX_TR));

    // Header row.
    lines.push(table_row(headers, &widths, alignments));

    // Header/data separator.
    lines.push(h_border(&widths, BOX_ML, BOX_MM, BOX_MR));

    // Data rows.
    for row in rows {
        let cells: Vec<&str> = (0..ncols)
            .map(|i| row.get(i).map(String::as_str).unwrap_or(""))
            .collect();
        lines.push(table_row(&cells, &widths, alignments));
    }

    // Bottom border.
    lines.push(h_border(&widths, BOX_BL, BOX_BM, BOX_BR));

    // End-of-report marker.
    lines.push(String::new());
    let marker = "— End of Report —";
    let table_width: usize = lines[0].chars().count();
    let marker_len = marker.chars().count();
    let left_pad = table_width.saturating_sub(marker_len) / 2;
    lines.push(format!("{}{}", " ".repeat(left_pad), marker));

    lines.join("\n")
}

/// Writes report content to `{output_dir}/{name}{MM-DD-YYYY}.txt`.
///
/// Creates `output_dir` (and all parents) if they do not exist.
/// Returns the full path of the created file.
pub fn write_report(content: &str, name: &str, output_dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(output_dir)
        .map_err(|e| anyhow::anyhow!("Failed to create report directory: {}", e))?;
    let date_str = chrono::Local::now().format("%m-%d-%Y").to_string();
    let filename = format!("{}{}.txt", name, date_str);
    let path = output_dir.join(&filename);
    std::fs::write(&path, content)
        .map_err(|e| anyhow::anyhow!("Failed to write report '{}': {}", path.display(), e))?;
    Ok(path)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── format_money ─────────────────────────────────────────────────────────

    #[test]
    fn format_money_positive() {
        assert_eq!(format_money(Money::from_dollars(1_234.56)), "1,234.56");
    }

    #[test]
    fn format_money_zero() {
        assert_eq!(format_money(Money::from_dollars(0.0)), "0.00");
    }

    #[test]
    fn format_money_negative() {
        assert_eq!(format_money(Money::from_dollars(-42.50)), "-42.50");
    }

    #[test]
    fn format_money_large() {
        assert_eq!(
            format_money(Money::from_dollars(1_000_000.00)),
            "1,000,000.00"
        );
    }

    // ── format_header ─────────────────────────────────────────────────────────

    #[test]
    fn format_header_contains_corner_chars() {
        let h = format_header("Acme LLC", "Trial Balance", "As of 2026-03-31");
        assert!(h.contains('┌'), "missing TL corner");
        assert!(h.contains('┐'), "missing TR corner");
        assert!(h.contains('└'), "missing BL corner");
        assert!(h.contains('┘'), "missing BR corner");
    }

    #[test]
    fn format_header_contains_content() {
        let h = format_header("Acme LLC", "Balance Sheet", "As of 2026-03-31");
        assert!(h.contains("Acme LLC"), "entity name missing");
        assert!(h.contains("Balance Sheet"), "title missing");
        assert!(h.contains("As of 2026-03-31"), "date info missing");
        assert!(h.contains("Accrual Basis"), "basis line missing");
        assert!(h.contains("Generated:"), "generated timestamp missing");
    }

    #[test]
    fn format_header_all_lines_equal_char_width() {
        let h = format_header("Acme Land LLC", "Income Statement", "Jan 1 – Dec 31, 2026");
        let lines: Vec<&str> = h.lines().collect();
        assert!(
            lines.len() >= 7,
            "expected at least 7 lines (top + 5 content + bottom)"
        );
        let widths: Vec<usize> = lines.iter().map(|l| l.chars().count()).collect();
        let first = widths[0];
        assert!(
            widths.iter().all(|&w| w == first),
            "lines have unequal widths: {:?}",
            widths
        );
    }

    #[test]
    fn format_header_expands_for_long_content() {
        let long_entity = "A Very Long Entity Name That Exceeds The Minimum Width";
        let h = format_header(long_entity, "Report", "2026");
        assert!(h.contains(long_entity), "long entity name should appear");
        // Every line should be at least as wide as the entity name + box chars.
        let min_width = long_entity.len() + 2; // 2 box chars
        for line in h.lines() {
            assert!(
                line.chars().count() >= min_width,
                "line too short: {}",
                line
            );
        }
    }

    // ── format_table ─────────────────────────────────────────────────────────

    #[test]
    fn format_table_contains_headers() {
        let headers = ["Account", "Debit", "Credit"];
        let rows: Vec<Vec<String>> = vec![];
        let alignments = [Align::Left, Align::Right, Align::Right];
        let t = format_table(&headers, &rows, &alignments);
        assert!(t.contains("Account"));
        assert!(t.contains("Debit"));
        assert!(t.contains("Credit"));
    }

    #[test]
    fn format_table_contains_data_rows() {
        let headers = ["Account", "Amount"];
        let rows = vec![
            vec!["Cash".to_owned(), "500.00".to_owned()],
            vec!["Revenue".to_owned(), "500.00".to_owned()],
        ];
        let alignments = [Align::Left, Align::Right];
        let t = format_table(&headers, &rows, &alignments);
        assert!(t.contains("Cash"));
        assert!(t.contains("Revenue"));
        assert!(t.contains("500.00"));
    }

    #[test]
    fn format_table_right_aligned_cell_is_right_padded() {
        // A right-aligned cell "42" in a column of width 6 should look like " "    42 "
        let headers = ["Num"];
        let rows = vec![vec!["42".to_owned()]];
        let alignments = [Align::Right];
        let t = format_table(&headers, &rows, &alignments);
        // The "42" cell should not be immediately followed by trailing spaces in the data row.
        // It should be right-aligned: "│  42 │" (space left, right-aligned, space right separator)
        // Check that "42" appears before the closing │ with at most 1 space.
        assert!(
            t.contains("42 │") || t.contains("42│"),
            "42 should be right-aligned near │"
        );
    }

    #[test]
    fn format_table_auto_expands_column_for_wide_data() {
        // Header "X" is 1 char. Data cell "LongValue" is 9 chars.
        // Column width should expand to 9.
        let headers = ["X"];
        let rows = vec![vec!["LongValue".to_owned()]];
        let alignments = [Align::Left];
        let t = format_table(&headers, &rows, &alignments);
        assert!(
            t.contains("LongValue"),
            "wide data cell should appear in output"
        );
        // The header separator line must be at least as wide as "LongValue" + padding.
        let separator = t.lines().nth(2).expect("separator line missing");
        assert!(
            separator.chars().count() >= "LongValue".len() + 4,
            "separator too narrow: {}",
            separator
        );
    }

    #[test]
    fn format_table_has_top_and_bottom_border() {
        let headers = ["Col"];
        let rows: Vec<Vec<String>> = vec![];
        let alignments = [Align::Left];
        let t = format_table(&headers, &rows, &alignments);
        let lines: Vec<&str> = t.lines().collect();
        assert!(lines[0].contains('┌'), "top-left corner missing");
        assert!(lines[0].contains('┐'), "top-right corner missing");
        // The bottom border is before the blank line and end-of-report marker.
        let border_line = lines
            .iter()
            .rfind(|l| l.contains('└'))
            .expect("no bottom border");
        assert!(border_line.contains('└'), "bottom-left corner missing");
        assert!(border_line.contains('┘'), "bottom-right corner missing");
    }

    #[test]
    fn format_table_has_end_of_report_marker() {
        let headers = ["Col"];
        let rows: Vec<Vec<String>> = vec![];
        let alignments = [Align::Left];
        let t = format_table(&headers, &rows, &alignments);
        assert!(
            t.contains("— End of Report —"),
            "end-of-report marker missing"
        );
    }

    #[test]
    fn format_table_box_lines_equal_char_width() {
        let headers = ["Account", "Debit", "Credit"];
        let rows = vec![
            vec!["Cash".to_owned(), "1,000.00".to_owned(), "0.00".to_owned()],
            vec![
                "Revenue".to_owned(),
                "0.00".to_owned(),
                "1,000.00".to_owned(),
            ],
        ];
        let alignments = [Align::Left, Align::Right, Align::Right];
        let t = format_table(&headers, &rows, &alignments);
        // Only check box-drawing lines (ignore trailing blank + end-of-report marker).
        let widths: Vec<usize> = t
            .lines()
            .filter(|l| {
                l.starts_with('┌') || l.starts_with('│') || l.starts_with('├') || l.starts_with('└')
            })
            .map(|l| l.chars().count())
            .collect();
        let first = widths[0];
        assert!(
            widths.iter().all(|&w| w == first),
            "table box lines have unequal widths: {:?}",
            widths
        );
    }

    // ── write_report ─────────────────────────────────────────────────────────

    #[test]
    fn write_report_creates_file_with_correct_content() {
        let dir = std::env::temp_dir().join("bursar_report_test");
        let content = "Report content here.";
        let path = write_report(content, "TestReport", &dir).expect("write_report failed");

        assert!(path.exists(), "report file should exist");
        let read_back = std::fs::read_to_string(&path).expect("read failed");
        assert_eq!(read_back, content);

        // Filename should start with "TestReport" and end with ".txt".
        let filename = path.file_name().unwrap().to_string_lossy();
        assert!(
            filename.starts_with("TestReport"),
            "bad prefix: {}",
            filename
        );
        assert!(filename.ends_with(".txt"), "bad extension: {}", filename);

        // Cleanup.
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn write_report_creates_output_dir_if_missing() {
        let dir = std::env::temp_dir()
            .join("bursar_report_dir_create_test")
            .join("subdir");
        // Ensure the directory does not exist.
        let _ = std::fs::remove_dir_all(&dir);

        let path = write_report("hello", "Test", &dir).expect("write_report failed");
        assert!(dir.exists(), "output dir should have been created");
        assert!(path.exists(), "report file should exist");

        // Cleanup.
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn write_report_filename_includes_date() {
        let dir = std::env::temp_dir().join("bursar_report_date_test");
        let path = write_report("x", "MyReport", &dir).expect("write_report failed");
        let filename = path.file_name().unwrap().to_string_lossy().to_string();
        // Date portion is MM-DD-YYYY: two digits, dash, two digits, dash, four digits.
        // e.g. "MyReport03-16-2026.txt"
        let date_part = filename
            .strip_prefix("MyReport")
            .and_then(|s| s.strip_suffix(".txt"))
            .expect("filename format wrong");
        assert_eq!(
            date_part.len(),
            10,
            "date should be MM-DD-YYYY: got '{}'",
            date_part
        );
        assert_eq!(&date_part[2..3], "-", "first dash missing");
        assert_eq!(&date_part[5..6], "-", "second dash missing");

        // Cleanup.
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
}
