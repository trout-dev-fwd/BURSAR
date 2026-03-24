use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::db::EntityDb;
use crate::tabs::{Tab, TabAction};
use crate::types::TaxFormTag;

// ── Form config modal state ───────────────────────────────────────────────────

/// State for the `c` key form configuration modal.
struct FormConfigModal {
    /// Toggle state for each form in `TaxFormTag::all()` order.
    enabled: Vec<bool>,
    /// Currently highlighted row.
    cursor: usize,
}

impl FormConfigModal {
    fn new(enabled_forms: &[TaxFormTag]) -> Self {
        let all = TaxFormTag::all();
        let enabled: Vec<bool> = all.iter().map(|f| enabled_forms.contains(f)).collect();
        Self { enabled, cursor: 0 }
    }

    /// Returns the list of currently-enabled form tags.
    fn as_enabled_list(&self) -> Vec<TaxFormTag> {
        TaxFormTag::all()
            .into_iter()
            .enumerate()
            .filter_map(|(i, f)| if self.enabled[i] { Some(f) } else { None })
            .collect()
    }

    fn handle_key(&mut self, key: KeyEvent) -> FormConfigAction {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
                FormConfigAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.cursor + 1 < TaxFormTag::all().len() {
                    self.cursor += 1;
                }
                FormConfigAction::None
            }
            KeyCode::Char(' ') => {
                self.enabled[self.cursor] = !self.enabled[self.cursor];
                FormConfigAction::None
            }
            KeyCode::Enter => FormConfigAction::Save(self.as_enabled_list()),
            KeyCode::Esc => FormConfigAction::Cancel,
            _ => FormConfigAction::None,
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let all = TaxFormTag::all();
        let row_count = all.len();
        let popup_height = (row_count + 4).min(area.height as usize) as u16;
        let popup_width = 60u16.min(area.width);

        let x = area.x + area.width.saturating_sub(popup_width) / 2;
        let y = area.y + area.height.saturating_sub(popup_height) / 2;
        let popup_area = Rect::new(x, y, popup_width, popup_height);

        let block = Block::default()
            .title(" Configure Tax Forms (Space: toggle, Enter: save, Esc: cancel) ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Cyan).bg(Color::Black));

        let inner = block.inner(popup_area);

        let lines: Vec<Line> = all
            .iter()
            .enumerate()
            .map(|(i, form)| {
                let check = if self.enabled[i] { "[✓]" } else { "[ ]" };
                let is_selected = i == self.cursor;
                let style = if is_selected {
                    Style::default()
                        .fg(Color::Yellow)
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                Line::from(vec![
                    Span::styled(format!(" {check} "), style),
                    Span::styled(form.display_name(), style),
                ])
            })
            .collect();

        frame.render_widget(Clear, popup_area);
        frame.render_widget(block, popup_area);
        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(Color::Black)),
            inner,
        );
    }
}

enum FormConfigAction {
    None,
    Save(Vec<TaxFormTag>),
    Cancel,
}

// ── TaxTab ────────────────────────────────────────────────────────────────────

/// Tax workstation tab — Phase 3 will add the JE list view and full workflow.
pub struct TaxTab {
    /// Currently enabled tax forms. Initialized to all-enabled.
    enabled_forms: Vec<TaxFormTag>,
    /// Active form configuration modal (Some when `c` is pressed).
    form_config_modal: Option<FormConfigModal>,
}

impl TaxTab {
    pub fn new() -> Self {
        Self {
            enabled_forms: TaxFormTag::all(),
            form_config_modal: None,
        }
    }

    /// Updates enabled forms from a saved list of tag strings.
    /// Call this after loading the entity TOML to restore persisted config.
    pub fn set_enabled_forms_from_strings(&mut self, form_strings: &[String]) {
        self.enabled_forms = TaxFormTag::all()
            .into_iter()
            .filter(|f| form_strings.contains(&f.to_string()))
            .collect();
        // If nothing matched, default to all enabled.
        if self.enabled_forms.is_empty() {
            self.enabled_forms = TaxFormTag::all();
        }
    }

    /// Returns the currently enabled forms.
    pub fn enabled_forms(&self) -> &[TaxFormTag] {
        &self.enabled_forms
    }
}

impl Default for TaxTab {
    fn default() -> Self {
        Self::new()
    }
}

impl Tab for TaxTab {
    fn title(&self) -> &str {
        "Tax"
    }

    fn handle_key(&mut self, key: KeyEvent, _db: &EntityDb) -> TabAction {
        if let Some(ref mut modal) = self.form_config_modal {
            match modal.handle_key(key) {
                FormConfigAction::None => {}
                FormConfigAction::Save(forms) => {
                    self.enabled_forms = forms.clone();
                    self.form_config_modal = None;
                    let tags: Vec<String> = forms.iter().map(|f| f.to_string()).collect();
                    return TabAction::SaveTaxFormConfig(tags);
                }
                FormConfigAction::Cancel => {
                    self.form_config_modal = None;
                }
            }
            return TabAction::None;
        }

        match key.code {
            KeyCode::Char('c') => {
                self.form_config_modal = Some(FormConfigModal::new(&self.enabled_forms));
                TabAction::None
            }
            KeyCode::Char('u') => TabAction::StartTaxIngestion,
            _ => TabAction::None,
        }
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Tax Workstation ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new("Tax workstation — coming soon.")
                .style(Style::default().fg(Color::DarkGray)),
            inner,
        );

        if let Some(ref modal) = self.form_config_modal {
            modal.render(frame, area);
        }
    }

    fn refresh(&mut self, _db: &EntityDb) {}

    fn wants_input(&self) -> bool {
        self.form_config_modal.is_some()
    }

    fn hotkey_help(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("c", "Configure enabled tax forms"),
            ("u", "Update tax reference library (fetch IRS publications)"),
        ]
    }
}
