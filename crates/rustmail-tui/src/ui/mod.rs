mod help;
mod list;
mod preview;
pub mod util;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::{App, Mode};

pub fn render(frame: &mut Frame, app: &App) {
  match app.mode {
    Mode::RawView => render_raw_view(frame, app),
    _ => render_main(frame, app),
  }
}

fn render_main(frame: &mut Frame, app: &App) {
  let area = frame.area();

  let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints([Constraint::Min(0), Constraint::Length(1)])
    .split(area);

  let main_area = chunks[0];
  let help_area = chunks[1];

  let panes = Layout::default()
    .direction(Direction::Horizontal)
    .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
    .split(main_area);

  list::render(frame, app, panes[0]);
  preview::render(frame, app, panes[1]);
  help::render(frame, app, help_area);

  if app.mode == Mode::Search {
    render_search_input(frame, app, area);
  }

  if app.mode == Mode::Confirm {
    render_confirm_dialog(frame, area);
  }

  if let Some(ref err) = app.error {
    render_error(frame, err, area);
  }
}

fn render_raw_view(frame: &mut Frame, app: &App) {
  let area = frame.area();

  let content = app.raw_content.as_deref().unwrap_or("");

  let block = Block::default()
    .title(" Raw Message (RFC 5322) ")
    .title_alignment(Alignment::Center)
    .borders(Borders::ALL)
    .border_style(Style::default().fg(Color::DarkGray));

  let paragraph = Paragraph::new(content)
    .block(block)
    .wrap(Wrap { trim: false })
    .scroll((app.raw_scroll, 0));

  frame.render_widget(paragraph, area);

  let help =
    Paragraph::new(" q/Esc: close  j/k: scroll ").style(Style::default().fg(Color::DarkGray));
  let help_area = Rect::new(
    area.x,
    area.y + area.height.saturating_sub(1),
    area.width,
    1,
  );
  frame.render_widget(help, help_area);
}

fn render_search_input(frame: &mut Frame, app: &App, area: Rect) {
  let width = 50u16.min(area.width.saturating_sub(4));
  let height = 3u16;
  let x = (area.width.saturating_sub(width)) / 2;
  let y = (area.height.saturating_sub(height)) / 2;
  let popup_area = Rect::new(x, y, width, height);

  frame.render_widget(Clear, popup_area);

  let block = Block::default()
    .title(" Search ")
    .borders(Borders::ALL)
    .border_style(Style::default().fg(Color::Cyan));

  let input = Paragraph::new(app.search_input.as_str()).block(block);
  frame.render_widget(input, popup_area);

  let cursor_x = app.search_input.chars().count() as u16;
  frame.set_cursor_position((
    popup_area.x + 1 + cursor_x.min(popup_area.width.saturating_sub(2)),
    popup_area.y + 1,
  ));
}

fn render_confirm_dialog(frame: &mut Frame, area: Rect) {
  let width = 40u16.min(area.width.saturating_sub(4));
  let height = 5u16;
  let x = (area.width.saturating_sub(width)) / 2;
  let y = (area.height.saturating_sub(height)) / 2;
  let popup_area = Rect::new(x, y, width, height);

  frame.render_widget(Clear, popup_area);

  let block = Block::default()
    .title(" Confirm ")
    .borders(Borders::ALL)
    .border_style(Style::default().fg(Color::Red));

  let text = vec![
    Line::from("Delete ALL messages?"),
    Line::from(""),
    Line::from(Span::styled(
      "y: yes  any other key: cancel",
      Style::default().fg(Color::DarkGray),
    )),
  ];

  let paragraph = Paragraph::new(text)
    .block(block)
    .alignment(Alignment::Center);
  frame.render_widget(paragraph, popup_area);
}

fn render_error(frame: &mut Frame, err: &str, area: Rect) {
  let err_len = (err.len()).min(u16::MAX as usize) as u16;
  let width = err_len.saturating_add(4).min(area.width.saturating_sub(4));
  let popup_area = Rect::new((area.width.saturating_sub(width)) / 2, 0, width, 3);

  frame.render_widget(Clear, popup_area);

  let block = Block::default()
    .borders(Borders::ALL)
    .border_style(Style::default().fg(Color::Red));

  let paragraph = Paragraph::new(err)
    .block(block)
    .style(Style::default().fg(Color::Red));
  frame.render_widget(paragraph, popup_area);
}
