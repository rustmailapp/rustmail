use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::util::{format_size, truncate};
use crate::app::{App, Focus};

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
  let border_style = if app.focus == Focus::Preview {
    Style::default().fg(Color::Cyan)
  } else {
    Style::default().fg(Color::DarkGray)
  };

  let title = if app.preview_loading {
    " Loading... ".to_string()
  } else if let Some(ref msg) = app.preview {
    let subject = msg.subject.as_deref().unwrap_or("(no subject)");
    let max_len = area.width.saturating_sub(4) as usize;
    format!(" {} ", truncate(subject, max_len))
  } else {
    " Preview ".to_string()
  };

  let block = Block::default()
    .title(title)
    .borders(Borders::ALL)
    .border_style(border_style);

  let inner = block.inner(area);
  frame.render_widget(block, area);

  if app.preview_loading {
    let p = Paragraph::new("Loading...")
      .alignment(Alignment::Center)
      .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(p, inner);
    return;
  }

  let Some(ref msg) = app.preview else {
    let p = Paragraph::new("Select a message to preview")
      .alignment(Alignment::Center)
      .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(p, inner);
    return;
  };

  let recipients_str = msg.recipients.join(", ");

  let header_style = Style::default()
    .fg(Color::Cyan)
    .add_modifier(Modifier::BOLD);

  let mut lines: Vec<Line> = Vec::new();

  lines.push(Line::from(vec![
    Span::styled("From: ", header_style),
    Span::raw(&msg.sender),
  ]));

  lines.push(Line::from(vec![
    Span::styled("To:   ", header_style),
    Span::raw(&recipients_str),
  ]));

  if let Some(ref subject) = msg.subject {
    lines.push(Line::from(vec![
      Span::styled("Subj: ", header_style),
      Span::raw(subject.as_str()),
    ]));
  }

  lines.push(Line::from(vec![
    Span::styled("Date: ", header_style),
    Span::raw(&msg.created_at),
  ]));

  let size_str = format_size(msg.size);

  let mut meta_parts = vec![size_str];
  if msg.has_attachments {
    meta_parts.push("📎 attachments".into());
  }
  if msg.is_starred {
    meta_parts.push("★ starred".into());
  }
  if !msg.tags.is_empty() {
    meta_parts.push(format!("tags: {}", msg.tags.join(", ")));
  }

  lines.push(Line::from(vec![
    Span::styled("Info: ", header_style),
    Span::raw(meta_parts.join("  |  ")),
  ]));

  lines.push(Line::from(
    "─".repeat(inner.width.saturating_sub(1) as usize),
  ));
  lines.push(Line::from(""));

  let body = msg
    .text_body
    .as_deref()
    .or(msg.html_body.as_deref())
    .unwrap_or("(no body)");

  for line in body.lines() {
    lines.push(Line::from(line.to_string()));
  }

  let paragraph = Paragraph::new(lines)
    .wrap(Wrap { trim: false })
    .scroll((app.preview_scroll, 0));

  frame.render_widget(paragraph, inner);
}
