//! A navigable file browser modal for selecting a CSV file.
//!
//! Shows directories first (sorted), then `.csv` files (sorted). Hidden entries
//! (names starting with `.`) are excluded. The widget renders as a centered modal.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use super::centered_rect;

// ── Entry ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Entry {
    Dir(String),
    File(String),
}

impl Entry {
    fn name(&self) -> &str {
        match self {
            Entry::Dir(n) | Entry::File(n) => n,
        }
    }
}

// ── FilePickerAction ──────────────────────────────────────────────────────────

/// Result of a key event handled by the file picker widget.
#[derive(Debug, Clone)]
pub enum FilePickerAction {
    /// User selected a `.csv` file; contains the full path.
    Selected(PathBuf),
    /// User cancelled (Esc).
    Cancelled,
    /// Key consumed; still waiting for a selection.
    Pending,
}

// ── FilePicker ────────────────────────────────────────────────────────────────

/// A navigable file browser modal.
///
/// Instantiate with [`FilePicker::new`], call [`handle_key`] on each key event,
/// and call [`render`] each frame until a non-`Pending` action is returned.
pub struct FilePicker {
    current_dir: PathBuf,
    entries: Vec<Entry>,
    selected_index: usize,
}

impl FilePicker {
    /// Creates a new file picker starting in `start_dir`.
    ///
    /// If `start_dir` is not a valid directory, falls back to the user's home
    /// directory, and then to the current working directory.
    pub fn new(start_dir: PathBuf) -> Self {
        let dir = if start_dir.is_dir() {
            start_dir
        } else {
            std::env::var("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("."))
        };
        let mut picker = Self {
            current_dir: dir,
            entries: Vec::new(),
            selected_index: 0,
        };
        picker.refresh_entries();
        picker
    }

    /// Scans the current directory and resets the selection.
    fn refresh_entries(&mut self) {
        self.entries = scan_dir(&self.current_dir);
        self.selected_index = 0;
    }

    /// Navigates into the named subdirectory.
    fn enter_dir(&mut self, name: String) {
        self.current_dir = self.current_dir.join(&name);
        self.refresh_entries();
    }

    /// Navigates to the parent directory (no-op at filesystem root).
    fn go_up(&mut self) {
        if let Some(parent) = self.current_dir.parent().map(Path::to_path_buf) {
            self.current_dir = parent;
            self.refresh_entries();
        }
    }

    /// Handles a key event and returns the resulting action.
    pub fn handle_key(&mut self, key: KeyEvent) -> FilePickerAction {
        match key.code {
            KeyCode::Esc => FilePickerAction::Cancelled,
            KeyCode::Backspace => {
                self.go_up();
                FilePickerAction::Pending
            }
            KeyCode::Up => {
                self.selected_index = self.selected_index.saturating_sub(1);
                FilePickerAction::Pending
            }
            KeyCode::Down => {
                if !self.entries.is_empty() && self.selected_index + 1 < self.entries.len() {
                    self.selected_index += 1;
                }
                FilePickerAction::Pending
            }
            KeyCode::Enter => {
                if let Some(entry) = self.entries.get(self.selected_index).cloned() {
                    match entry {
                        Entry::Dir(name) => {
                            self.enter_dir(name);
                            FilePickerAction::Pending
                        }
                        Entry::File(name) => {
                            FilePickerAction::Selected(self.current_dir.join(name))
                        }
                    }
                } else {
                    FilePickerAction::Pending
                }
            }
            _ => FilePickerAction::Pending,
        }
    }

    /// Renders the file picker modal centered within `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let modal = centered_rect(70, 60, area);
        frame.render_widget(Clear, modal);

        // Inner height: modal height minus 2 (borders) minus 3 (dir label, separator, hint)
        // minus 1 (blank line before hint) = modal.height - 6
        let visible_rows = modal.height.saturating_sub(6) as usize;
        let scroll = self.compute_scroll(visible_rows);

        let dir_label = self.current_dir.to_string_lossy();
        let mut lines: Vec<Line> = vec![
            Line::from(Span::styled(
                format!(" {dir_label}"),
                Style::default().fg(Color::Cyan),
            )),
            Line::from(Span::styled(
                " ─────────────────────────────────────────────────────",
                Style::default().fg(Color::DarkGray),
            )),
        ];

        if self.entries.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (no .csv files or subdirectories)",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            let end = (scroll + visible_rows).min(self.entries.len());
            for (i, entry) in self.entries[scroll..end].iter().enumerate() {
                let actual_idx = scroll + i;
                let is_selected = actual_idx == self.selected_index;
                let label = match entry {
                    Entry::Dir(n) => format!("  [dir] {n}/"),
                    Entry::File(n) => format!("        {n}"),
                };
                let base_color = match entry {
                    Entry::Dir(_) => Color::Yellow,
                    Entry::File(_) => Color::White,
                };
                let style = if is_selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(base_color)
                };
                lines.push(Line::from(Span::styled(label, style)));
            }
        }

        // Pad to fill visible area so the hint stays at the bottom.
        let content_rows = lines.len() - 2; // subtract dir + separator rows
        for _ in content_rows..visible_rows {
            lines.push(Line::from(Span::raw("")));
        }

        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(Span::styled(
            "  Enter: select  Backspace: up  Esc: cancel",
            Style::default().fg(Color::DarkGray),
        )));

        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Import CSV Statement ")
                    .style(Style::default().fg(Color::Cyan)),
            ),
            modal,
        );
    }

    /// Computes the scroll offset needed to keep `selected_index` visible.
    fn compute_scroll(&self, visible_rows: usize) -> usize {
        let visible = visible_rows.max(1);
        if self.selected_index < visible {
            0
        } else {
            self.selected_index - visible + 1
        }
    }
}

// ── scan_dir ──────────────────────────────────────────────────────────────────

/// Scans `dir` and returns entries: directories first (sorted A-Z),
/// then `.csv` files (sorted A-Z). Hidden entries (starting with `.`) are excluded.
fn scan_dir(dir: &Path) -> Vec<Entry> {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut dirs: Vec<Entry> = Vec::new();
    let mut files: Vec<Entry> = Vec::new();

    for entry in read_dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                dirs.push(Entry::Dir(name));
            } else if ft.is_file() && name.to_lowercase().ends_with(".csv") {
                files.push(Entry::File(name));
            }
        }
    }

    dirs.sort_by(|a, b| a.name().cmp(b.name()));
    files.sort_by(|a, b| a.name().cmp(b.name()));
    dirs.extend(files);
    dirs
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use std::fs;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn esc_cancels() {
        let dir = std::env::temp_dir();
        let mut picker = FilePicker::new(dir);
        assert!(matches!(
            picker.handle_key(key(KeyCode::Esc)),
            FilePickerAction::Cancelled
        ));
    }

    #[test]
    fn down_up_navigation() {
        let dir = std::env::temp_dir();
        let mut picker = FilePicker::new(dir);
        // Create a few entries by seeding entries directly for testing.
        picker.entries = vec![
            Entry::Dir("alpha".into()),
            Entry::Dir("beta".into()),
            Entry::File("data.csv".into()),
        ];
        assert_eq!(picker.selected_index, 0);
        picker.handle_key(key(KeyCode::Down));
        assert_eq!(picker.selected_index, 1);
        picker.handle_key(key(KeyCode::Down));
        assert_eq!(picker.selected_index, 2);
        // Past end — stays at 2.
        picker.handle_key(key(KeyCode::Down));
        assert_eq!(picker.selected_index, 2);
        picker.handle_key(key(KeyCode::Up));
        assert_eq!(picker.selected_index, 1);
    }

    #[test]
    fn enter_on_file_returns_selected() {
        let tmp = std::env::temp_dir();
        let csv_path = tmp.join("test_file_picker.csv");
        let _ = fs::write(&csv_path, "a,b,c");

        let mut picker = FilePicker::new(tmp.clone());
        // Force entries to our known file.
        picker.entries = vec![Entry::File("test_file_picker.csv".into())];
        picker.selected_index = 0;
        let action = picker.handle_key(key(KeyCode::Enter));
        let _ = fs::remove_file(&csv_path);
        assert!(matches!(action, FilePickerAction::Selected(_)));
        if let FilePickerAction::Selected(p) = action {
            assert_eq!(p.file_name().unwrap(), "test_file_picker.csv");
        }
    }

    #[test]
    fn enter_on_dir_navigates_into_it() {
        let tmp = std::env::temp_dir();
        let sub = tmp.join("file_picker_test_subdir");
        let _ = fs::create_dir_all(&sub);

        let mut picker = FilePicker::new(tmp.clone());
        picker.entries = vec![Entry::Dir("file_picker_test_subdir".into())];
        picker.selected_index = 0;
        picker.handle_key(key(KeyCode::Enter));
        let _ = fs::remove_dir(&sub);
        assert_eq!(picker.current_dir, sub);
    }

    #[test]
    fn backspace_goes_to_parent() {
        let tmp = std::env::temp_dir();
        let parent = tmp.parent().map(Path::to_path_buf).unwrap_or(tmp.clone());
        let mut picker = FilePicker::new(tmp);
        picker.handle_key(key(KeyCode::Backspace));
        assert_eq!(picker.current_dir, parent);
    }

    #[test]
    fn compute_scroll_keeps_selection_visible() {
        let dir = std::env::temp_dir();
        let mut picker = FilePicker::new(dir);
        picker.entries = (0..20)
            .map(|i| Entry::File(format!("file{i:02}.csv")))
            .collect();
        picker.selected_index = 15;
        // With 10 visible rows, scroll should be 6 (15 - 10 + 1).
        assert_eq!(picker.compute_scroll(10), 6);
        picker.selected_index = 3;
        assert_eq!(picker.compute_scroll(10), 0);
    }

    #[test]
    fn scan_dir_excludes_hidden() {
        let tmp = std::env::temp_dir().join("fp_test_hidden_scan");
        let _ = fs::create_dir_all(&tmp);
        let _ = fs::write(tmp.join(".hidden.csv"), "");
        let _ = fs::write(tmp.join("visible.csv"), "");
        let _ = fs::create_dir_all(tmp.join(".hiddendir"));
        let entries = scan_dir(&tmp);
        // Clean up before asserting to avoid leaving state.
        let _ = fs::remove_file(tmp.join(".hidden.csv"));
        let _ = fs::remove_file(tmp.join("visible.csv"));
        let _ = fs::remove_dir(tmp.join(".hiddendir"));
        let _ = fs::remove_dir(&tmp);
        for e in &entries {
            assert!(
                !e.name().starts_with('.'),
                "hidden entry leaked: {}",
                e.name()
            );
        }
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn scan_dir_dirs_before_files() {
        let tmp = std::env::temp_dir().join("fp_test_order_scan");
        let _ = fs::create_dir_all(&tmp);
        let _ = fs::write(tmp.join("aaa.csv"), "");
        let _ = fs::create_dir_all(tmp.join("zzz_dir"));
        let entries = scan_dir(&tmp);
        let _ = fs::remove_file(tmp.join("aaa.csv"));
        let _ = fs::remove_dir(tmp.join("zzz_dir"));
        let _ = fs::remove_dir(&tmp);
        assert!(!entries.is_empty(), "expected at least two entries");
        assert!(
            matches!(entries[0], Entry::Dir(_)),
            "dirs should come first"
        );
        assert!(
            matches!(entries[1], Entry::File(_)),
            "files should come after dirs"
        );
    }
}
