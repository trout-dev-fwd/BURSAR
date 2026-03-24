use crossterm::event::KeyEvent;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};

use crate::db::EntityDb;
use crate::tabs::{Tab, TabAction};

/// Tax workstation tab — Phase 3 will add the JE list view and full workflow.
pub struct TaxTab;

impl TaxTab {
    pub fn new() -> Self {
        Self
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

    fn handle_key(&mut self, _key: KeyEvent, _db: &EntityDb) -> TabAction {
        TabAction::None
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
    }

    fn refresh(&mut self, _db: &EntityDb) {}
}
