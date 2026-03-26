use ratatui::prelude::*;

pub struct Theme {
  pub border_focused: Style,
  pub border_unfocused: Style,
  pub border_error: Style,

  pub header_label: Style,
  pub header_value: Style,

  pub row_selected: Style,
  pub row_unread: Style,
  pub row_read: Style,

  pub tab_inactive: Style,
  pub tab_highlight: Style,

  pub help_desc: Style,

  pub status_ok: Style,
  pub status_err: Style,

  pub popup_border: Style,

  pub scrollbar_track: Style,
  pub scrollbar_thumb: Style,

  pub spinner: Style,
  pub empty_hint: Style,

}

pub const DEFAULT: Theme = Theme {
  border_focused: Style::new().fg(Color::Cyan),
  border_unfocused: Style::new().fg(Color::DarkGray),
  border_error: Style::new().fg(Color::Red),

  header_label: Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
  header_value: Style::new().fg(Color::White),

  row_selected: Style::new().bg(Color::DarkGray).fg(Color::White),
  row_unread: Style::new().add_modifier(Modifier::BOLD),
  row_read: Style::new().fg(Color::Gray),

  tab_inactive: Style::new().fg(Color::DarkGray),
  tab_highlight: Style::new()
    .fg(Color::Cyan)
    .add_modifier(Modifier::BOLD)
    .add_modifier(Modifier::UNDERLINED),

  help_desc: Style::new().fg(Color::DarkGray),

  status_ok: Style::new().fg(Color::Green),
  status_err: Style::new().fg(Color::Red),

  popup_border: Style::new().fg(Color::Cyan),

  scrollbar_track: Style::new().fg(Color::DarkGray),
  scrollbar_thumb: Style::new().fg(Color::Gray),

  spinner: Style::new().fg(Color::Cyan),
  empty_hint: Style::new().fg(Color::DarkGray),
};
