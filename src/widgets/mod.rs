pub mod account_picker;
pub mod chat_panel;
pub mod confirmation;
pub mod fiscal_modal;
pub mod je_form;
pub mod status_bar;
pub mod user_guide;

pub use account_picker::AccountPicker;
pub use confirmation::Confirmation;
pub use fiscal_modal::{FiscalModal, FiscalModalAction};
pub use je_form::JeForm;
pub use status_bar::StatusBar;
pub use user_guide::{UserGuide, UserGuideAction};

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Returns a centered `Rect` within `area` at `percent_x`% width and `percent_y`% height.
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let margin_x = (100 - percent_x) / 2;
    let margin_y = (100 - percent_y) / 2;
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(margin_x),
            Constraint::Percentage(percent_x),
            Constraint::Percentage(margin_x),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(margin_y),
            Constraint::Percentage(percent_y),
            Constraint::Percentage(margin_y),
        ])
        .split(horizontal[1])[1]
}
