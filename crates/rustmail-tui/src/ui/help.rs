use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::app::{App, Focus, Mode};

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
  if let Some(ref status) = app.status_message {
    let p =
      Paragraph::new(status.as_str()).style(Style::default().fg(Color::Yellow).bg(Color::DarkGray));
    frame.render_widget(p, area);
    return;
  }

  let help_text = match app.mode {
    Mode::Search => " Enter: search  Esc: cancel",
    Mode::Confirm => " y: confirm  any key: cancel",
    Mode::RawView => " q/Esc: close  j/k: scroll",
    Mode::Normal => match app.focus {
      Focus::List => {
        " j/k:nav  Enter:preview  /:search  r:read  s:star  d:del  D:clear  R:raw  [/]:page  ?:help  q:quit"
      }
      Focus::Preview => " j/k:scroll  h/Esc:list  r:read  s:star  d:del  R:raw  q:quit",
    },
  };

  let unread = app.unread_count();
  let right = if unread > 0 {
    format!(" {} unread ", unread)
  } else {
    String::new()
  };

  let right_display_width = right.chars().count() as u16;
  let left_width = area.width.saturating_sub(right_display_width) as usize;

  let help_truncated: String = help_text.chars().take(left_width).collect();
  let help_display_len = help_truncated.chars().count();
  let padding = left_width.saturating_sub(help_display_len);

  let spans = vec![
    Span::styled(
      help_truncated,
      Style::default().fg(Color::DarkGray).bg(Color::Black),
    ),
    Span::styled(" ".repeat(padding), Style::default().bg(Color::Black)),
    Span::styled(right, Style::default().fg(Color::Cyan).bg(Color::Black)),
  ];

  let p = Paragraph::new(Line::from(spans));
  frame.render_widget(p, area);
}
