use ratatui::buffer::Cell;
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::theme::Theme;

pub fn render_confirm(frame: &mut Frame, area: Rect, theme: &Theme) {
  render_dim_overlay(frame, area, theme);

  let width = 40u16.min(area.width.saturating_sub(4));
  let height = 5u16;
  let x = (area.width.saturating_sub(width)) / 2;
  let y = (area.height.saturating_sub(height)) / 2;
  let popup_area = Rect::new(x, y, width, height);

  frame.render_widget(Clear, popup_area);

  let block = Block::default()
    .title(" Confirm ")
    .borders(Borders::ALL)
    .border_type(BorderType::Rounded)
    .border_style(theme.border_error);

  let text = vec![
    Line::from("Delete ALL messages?"),
    Line::from(""),
    Line::from(Span::styled("y: yes  any key: cancel", theme.empty_hint)),
  ];

  let paragraph = Paragraph::new(text)
    .block(block)
    .alignment(Alignment::Center);
  frame.render_widget(paragraph, popup_area);
}

pub fn render_help(frame: &mut Frame, area: Rect, theme: &Theme) {
  render_dim_overlay(frame, area, theme);

  let shortcuts = [
    ("j / k", "Navigate up/down"),
    ("Enter / l", "Open preview"),
    ("h / Esc", "Back to list"),
    ("1 / 2 / 3", "Text / Headers / Raw tab"),
    ("/", "Search"),
    ("r", "Toggle read"),
    ("s", "Toggle star"),
    ("d", "Delete message"),
    ("D", "Delete all"),
    ("R", "Full raw view"),
    ("[ / ]", "Prev / next page"),
    ("g / G", "First / last message"),
    ("q", "Quit"),
  ];

  let key_col_width = 14u16;
  let desc_col_width = 24u16;
  let content_width = key_col_width + desc_col_width;
  let width = (content_width + 4).min(area.width.saturating_sub(4));
  let height = (shortcuts.len() as u16 + 2).min(area.height.saturating_sub(4));
  let x = (area.width.saturating_sub(width)) / 2;
  let y = (area.height.saturating_sub(height)) / 2;
  let popup_area = Rect::new(x, y, width, height);

  frame.render_widget(Clear, popup_area);

  let block = Block::default()
    .title(" Keyboard Shortcuts ")
    .title_bottom(
      Line::from(Span::styled(" Esc to close ", theme.help_desc)).alignment(Alignment::Center),
    )
    .borders(Borders::ALL)
    .border_type(BorderType::Rounded)
    .border_style(theme.popup_border);

  let lines: Vec<Line> = shortcuts
    .iter()
    .map(|(key, desc)| {
      Line::from(vec![
        Span::styled(
          format!("  {:<width$}", key, width = key_col_width as usize),
          theme.border_focused,
        ),
        Span::styled(*desc, theme.header_value),
      ])
    })
    .collect();

  let paragraph = Paragraph::new(lines).block(block);
  frame.render_widget(paragraph, popup_area);
}

pub fn render_error(frame: &mut Frame, err: &str, area: Rect, theme: &Theme) {
  let err_len = (err.len()).min(u16::MAX as usize) as u16;
  let width = err_len.saturating_add(4).min(area.width.saturating_sub(4));
  let popup_area = Rect::new((area.width.saturating_sub(width)) / 2, 0, width, 3);

  frame.render_widget(Clear, popup_area);

  let block = Block::default()
    .borders(Borders::ALL)
    .border_type(BorderType::Rounded)
    .border_style(theme.border_error);

  let paragraph = Paragraph::new(err).block(block).style(theme.border_error);
  frame.render_widget(paragraph, popup_area);
}

fn render_dim_overlay(frame: &mut Frame, area: Rect, _theme: &Theme) {
  let buf = frame.buffer_mut();
  for y in area.y..area.y + area.height {
    for x in area.x..area.x + area.width {
      let cell = &mut buf[(x, y)];
      dim_cell(cell);
    }
  }
}

fn dim_cell(cell: &mut Cell) {
  cell.set_fg(dim_color(cell.fg));
  cell.set_bg(dim_color(cell.bg));
}

fn dim_color(color: Color) -> Color {
  match color {
    Color::Rgb(r, g, b) => Color::Rgb(r / 3, g / 3, b / 3),
    Color::White => Color::DarkGray,
    Color::Gray => Color::DarkGray,
    Color::DarkGray => Color::Rgb(30, 30, 30),
    Color::Cyan => Color::Rgb(20, 50, 50),
    Color::Red => Color::Rgb(50, 15, 15),
    Color::Green => Color::Rgb(15, 50, 15),
    Color::Yellow => Color::Rgb(50, 50, 15),
    Color::Blue => Color::Rgb(15, 15, 50),
    Color::Magenta => Color::Rgb(50, 15, 50),
    Color::Reset => Color::Rgb(20, 20, 20),
    _ => Color::Rgb(30, 30, 30),
  }
}
