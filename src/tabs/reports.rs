use crossterm::event::KeyEvent;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    widgets::Paragraph,
};

use crate::db::EntityDb;
use crate::tabs::{RecordId, Tab, TabAction};

pub struct ReportsTab;

impl Tab for ReportsTab {
    fn title(&self) -> &str {
        "Reports"
    }

    fn handle_key(&mut self, _key: KeyEvent, _db: &EntityDb) -> TabAction {
        TabAction::None
    }

    fn render(&self, frame: &mut Frame, area: Rect) {
        frame.render_widget(
            Paragraph::new(self.title()).alignment(Alignment::Center),
            area,
        );
    }

    fn refresh(&mut self, _db: &EntityDb) {}

    fn navigate_to(&mut self, _record_id: RecordId, _db: &EntityDb) {}
}
