use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Paragraph, Scrollbar, ScrollbarOrientation, Wrap};

use super::util::{format_size, truncate};
use crate::app::{App, Focus, PreviewTab};
use crate::theme::Theme;

pub fn render(frame: &mut Frame, app: &mut App, area: Rect, theme: &Theme) {
  let border_style = if app.focus == Focus::Preview {
    theme.border_focused
  } else {
    theme.border_unfocused
  };

  let title = if app.preview_loading {
    format!(" {} Loading... ", app.spinner_char())
  } else if let Some(ref msg) = app.preview {
    let subject = msg.subject.as_deref().unwrap_or("(no subject)");
    let max_len = area.width.saturating_sub(4) as usize;
    format!(" {} ", truncate(subject, max_len))
  } else {
    " Preview ".to_string()
  };

  let block = Block::bordered()
    .title(title)
    .border_type(BorderType::Rounded)
    .border_style(border_style);

  let inner = block.inner(area);
  frame.render_widget(block, area);

  if app.preview_loading {
    let p = Paragraph::new(format!(" {} Loading...", app.spinner_char()))
      .alignment(Alignment::Center)
      .style(theme.spinner);
    frame.render_widget(p, inner);
    return;
  }

  let Some(ref msg) = app.preview else {
    let p = Paragraph::new("Select a message to preview")
      .alignment(Alignment::Center)
      .style(theme.empty_hint);
    frame.render_widget(p, inner);
    return;
  };

  let tab_chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);

  app.tab_area = tab_chunks[0];
  let content_area = tab_chunks[1];

  let tab_titles = [
    ("1 Text", PreviewTab::Text),
    ("2 Headers", PreviewTab::Headers),
    ("3 Raw", PreviewTab::Raw),
  ];
  let mut spans: Vec<Span> = Vec::new();
  let mut x_offset: u16 = 0;

  for (i, (label, tab)) in tab_titles.iter().enumerate() {
    if i > 0 {
      spans.push(Span::styled(" ", theme.tab_inactive));
      x_offset += 1;
    }
    let start = x_offset;
    if app.preview_tab == *tab {
      spans.push(Span::styled("", theme.tab_pill_edge));
      spans.push(Span::styled(format!(" {label} "), theme.tab_highlight));
      spans.push(Span::styled("", theme.tab_pill_edge));
      x_offset += 2 + label.len() as u16 + 2;
    } else {
      spans.push(Span::styled(format!(" {label} "), theme.tab_inactive));
      x_offset += label.len() as u16 + 2;
    }
    app.tab_ranges[i] = (start, x_offset);
  }

  let tabs_line = Paragraph::new(Line::from(spans));
  frame.render_widget(tabs_line, tab_chunks[0]);

  let lines = match app.preview_tab {
    PreviewTab::Text => render_text_lines(msg, content_area.width, theme),
    PreviewTab::Headers => render_header_lines(msg, theme),
    PreviewTab::Raw => render_raw_lines(app, theme),
  };

  let content_len = lines.len();
  let max_scroll = content_len.saturating_sub(content_area.height as usize) as u16;
  app.preview_scroll = app.preview_scroll.min(max_scroll);

  app.preview_scrollbar_state = app
    .preview_scrollbar_state
    .content_length(content_len)
    .position(app.preview_scroll as usize);

  let paragraph = Paragraph::new(lines)
    .wrap(Wrap { trim: false })
    .scroll((app.preview_scroll, 0));

  frame.render_widget(paragraph, content_area);

  if content_len > content_area.height as usize {
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
      .track_style(theme.scrollbar_track)
      .thumb_style(theme.scrollbar_thumb);

    frame.render_stateful_widget(scrollbar, content_area, &mut app.preview_scrollbar_state);
  }
}

fn render_text_lines(msg: &crate::api::Message, width: u16, theme: &Theme) -> Vec<Line<'static>> {
  let mut lines: Vec<Line> = Vec::new();

  let recipients_str = msg.recipients.join(", ");

  lines.push(Line::from(vec![
    Span::styled("From: ", theme.header_label),
    Span::styled(msg.sender.clone(), theme.header_value),
  ]));

  lines.push(Line::from(vec![
    Span::styled("To:   ", theme.header_label),
    Span::styled(recipients_str, theme.header_value),
  ]));

  if let Some(ref subject) = msg.subject {
    lines.push(Line::from(vec![
      Span::styled("Subj: ", theme.header_label),
      Span::styled(subject.clone(), theme.header_value),
    ]));
  }

  lines.push(Line::from(vec![
    Span::styled("Date: ", theme.header_label),
    Span::styled(msg.created_at.clone(), theme.header_value),
  ]));

  let size_str = format_size(msg.size);
  let mut meta_parts = vec![size_str];
  if msg.has_attachments {
    meta_parts.push("@ attachments".into());
  }
  if msg.is_starred {
    meta_parts.push("★ starred".into());
  }
  if !msg.tags.is_empty() {
    meta_parts.push(format!("tags: {}", msg.tags.join(", ")));
  }

  lines.push(Line::from(vec![
    Span::styled("Info: ", theme.header_label),
    Span::styled(meta_parts.join("  │  "), theme.header_value),
  ]));

  lines.push(Line::from("─".repeat(width.saturating_sub(1) as usize)));
  lines.push(Line::from(""));

  let body = if let Some(ref text) = msg.text_body {
    text.clone()
  } else if let Some(ref html) = msg.html_body {
    html2text::from_read(html.as_bytes(), width.saturating_sub(2).max(40) as usize)
      .unwrap_or_else(|_| html.clone())
  } else {
    "(no body)".to_string()
  };

  for line in body.lines() {
    lines.push(Line::from(String::from(line)));
  }

  lines
}

fn render_header_lines(msg: &crate::api::Message, theme: &Theme) -> Vec<Line<'static>> {
  let mut lines: Vec<Line> = Vec::new();

  let headers = [
    ("From", msg.sender.as_str()),
    ("To", &msg.recipients.join(", ")),
    ("Subject", msg.subject.as_deref().unwrap_or("(none)")),
    ("Date", msg.created_at.as_str()),
    ("Size", &format_size(msg.size)),
    ("Read", if msg.is_read { "Yes" } else { "No" }),
    ("Starred", if msg.is_starred { "Yes" } else { "No" }),
    (
      "Attachments",
      if msg.has_attachments { "Yes" } else { "No" },
    ),
  ];

  for (label, value) in headers {
    lines.push(Line::from(vec![
      Span::styled(format!("{:<12} ", label), theme.header_label),
      Span::styled(value.to_string(), theme.header_value),
    ]));
  }

  if !msg.tags.is_empty() {
    lines.push(Line::from(vec![
      Span::styled(format!("{:<12} ", "Tags"), theme.header_label),
      Span::styled(msg.tags.join(", "), theme.header_value),
    ]));
  }

  if let Some(ref text) = msg.text_body {
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Has text body", theme.empty_hint)));
    lines.push(Line::from(Span::styled(
      format!("  {} chars", text.len()),
      theme.empty_hint,
    )));
  }

  if let Some(ref html) = msg.html_body {
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Has HTML body", theme.empty_hint)));
    lines.push(Line::from(Span::styled(
      format!("  {} chars", html.len()),
      theme.empty_hint,
    )));
  }

  lines
}

fn render_raw_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
  match &app.preview_raw {
    Some(raw) => raw.lines().map(|l| Line::from(l.to_string())).collect(),
    None => vec![Line::from(Span::styled(
      format!(" {} Loading raw content...", app.spinner_char()),
      theme.empty_hint,
    ))],
  }
}
