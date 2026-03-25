use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};

use super::util::{format_date, format_size, parse_sender, truncate};
use crate::app::{App, Focus};

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
  let border_style = if app.focus == Focus::List {
    Style::default().fg(Color::Cyan)
  } else {
    Style::default().fg(Color::DarkGray)
  };

  let title = if app.search_query.is_empty() {
    format!(" Messages ({}) ", app.total)
  } else {
    format!(" Messages ({}) [search: {}] ", app.total, app.search_query)
  };

  let page_info = if app.total_pages() > 1 {
    format!(" {}/{} ", app.current_page(), app.total_pages())
  } else {
    String::new()
  };

  let block = Block::default()
    .title(title)
    .title_bottom(Line::from(page_info).alignment(Alignment::Right))
    .borders(Borders::ALL)
    .border_style(border_style);

  let inner = block.inner(area);
  frame.render_widget(block, area);

  if app.messages.is_empty() {
    let empty = if app.loading {
      "Loading..."
    } else {
      "No messages"
    };
    let p = ratatui::widgets::Paragraph::new(empty)
      .alignment(Alignment::Center)
      .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(p, inner);
    return;
  }

  let sender_width = (inner.width as f64 * 0.35) as u16;
  let date_width = 12u16;
  let size_width = 6u16;
  let icon_width = 4u16;
  let subject_width = inner
    .width
    .saturating_sub(sender_width + date_width + size_width + icon_width + 4);

  let widths = [
    Constraint::Length(icon_width),
    Constraint::Length(sender_width),
    Constraint::Length(subject_width),
    Constraint::Length(size_width),
    Constraint::Length(date_width),
  ];

  let header = Row::new(vec!["", "From", "Subject", "Size", "Date"])
    .style(
      Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::BOLD),
    )
    .bottom_margin(0);

  let rows: Vec<Row> = app
    .messages
    .iter()
    .enumerate()
    .map(|(i, msg)| {
      let is_selected = i == app.selected && app.focus == Focus::List;

      let read_icon = if msg.is_read { " " } else { "●" };
      let star_icon = if msg.is_starred { "★" } else { " " };
      let attach_icon = if msg.has_attachments { "📎" } else { " " };

      let row_style = if is_selected {
        Style::default().bg(Color::DarkGray).fg(Color::White)
      } else if !msg.is_read {
        Style::default().add_modifier(Modifier::BOLD)
      } else {
        Style::default().fg(Color::Gray)
      };

      let icon_style = if msg.is_starred && !is_selected {
        Style::default().fg(Color::Yellow)
      } else {
        row_style
      };

      let icons = format!("{}{}{}", read_icon, star_icon, attach_icon);
      let sender = parse_sender(&msg.sender);
      let subject = msg.subject.as_deref().unwrap_or("(no subject)");
      let size = format_size(msg.size);
      let date = format_date(&msg.created_at);

      Row::new(vec![
        Cell::from(icons).style(icon_style),
        Cell::from(truncate(&sender, sender_width as usize)),
        Cell::from(truncate(subject, subject_width as usize)),
        Cell::from(size),
        Cell::from(date),
      ])
      .style(row_style)
    })
    .collect();

  let table = Table::new(rows, widths).header(header);

  frame.render_widget(table, inner);
}
