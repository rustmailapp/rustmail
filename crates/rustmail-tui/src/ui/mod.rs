mod list;
mod popups;
mod preview;
pub mod util;

use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Paragraph, Wrap};

use crate::app::{App, Focus, Mode};
use crate::theme;

pub fn render(frame: &mut Frame, app: &mut App) {
  let theme = &theme::DEFAULT;

  match app.mode {
    Mode::RawView => render_raw_view(frame, app, theme),
    _ => render_main(frame, app, theme),
  }
}

fn render_main(frame: &mut Frame, app: &mut App, theme: &theme::Theme) {
  let area = frame.area();

  if area.width < 80 {
    match app.focus {
      Focus::List => {
        app.list_area = area;
        app.preview_area = Rect::default();
        list::render(frame, app, area, theme);
      }
      Focus::Preview => {
        app.list_area = Rect::default();
        app.preview_area = area;
        preview::render(frame, app, area, theme);
      }
    }
  } else {
    let list_pct = if area.width >= 120 { 35 } else { 40 };
    let panes = Layout::horizontal([
      Constraint::Percentage(list_pct),
      Constraint::Percentage(100 - list_pct),
    ])
    .split(area);

    app.list_area = panes[0];
    app.preview_area = panes[1];

    list::render(frame, app, panes[0], theme);
    preview::render(frame, app, panes[1], theme);
  }

  if app.mode == Mode::Confirm {
    popups::render_confirm(frame, area, theme);
  }

  if app.mode == Mode::Help {
    popups::render_help(frame, area, theme);
  }

  if let Some(err) = app.error.as_deref() {
    popups::render_error(frame, err, area, theme);
  }
}

fn render_raw_view(frame: &mut Frame, app: &App, theme: &theme::Theme) {
  let area = frame.area();

  let content = app.raw_content.as_deref().unwrap_or("");

  let block = Block::bordered()
    .title(Line::from(" Raw Message (RFC 5322) ").centered())
    .title_bottom(Line::from(Span::styled(" q:close  j/k:scroll ", theme.help_desc)).centered())
    .border_type(BorderType::Rounded)
    .border_style(theme.border_unfocused);

  let paragraph = Paragraph::new(content)
    .block(block)
    .wrap(Wrap { trim: false })
    .scroll((app.raw_scroll, 0));

  frame.render_widget(paragraph, area);
}
