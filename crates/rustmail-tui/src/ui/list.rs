use ratatui::prelude::*;
use ratatui::widgets::{
  Block, BorderType, Borders, HighlightSpacing, List, ListItem, Paragraph, Scrollbar,
  ScrollbarOrientation,
};

use super::util::{display_width, format_date, format_size, parse_sender, truncate};
use crate::app::{App, Focus, Mode};
use crate::theme::Theme;

pub fn render(frame: &mut Frame, app: &mut App, area: Rect, theme: &Theme) {
  let border_style = if app.focus == Focus::List {
    theme.border_focused
  } else {
    theme.border_unfocused
  };

  let title = format!(" Messages ({}) ", app.total);

  let ws_icon = if app.ws_connected { "●" } else { "○" };
  let ws_style = if app.ws_connected {
    theme.status_ok
  } else {
    theme.status_err
  };

  let mut bottom_left_spans: Vec<Span> = Vec::new();
  bottom_left_spans.push(Span::styled(format!(" {ws_icon}"), ws_style));

  let unread = app.unread_count();
  if unread > 0 {
    bottom_left_spans.push(Span::styled(
      format!(" {unread} unread"),
      theme.border_focused,
    ));
  }

  bottom_left_spans.push(Span::styled(" ?:help ", theme.help_desc));

  let mut bottom_right_spans: Vec<Span> = Vec::new();
  if app.total_pages() > 1 {
    bottom_right_spans.push(Span::styled(
      format!(" {}/{} ", app.current_page(), app.total_pages()),
      border_style,
    ));
  }

  let block = Block::default()
    .title(title)
    .title_bottom(Line::from(bottom_left_spans).alignment(Alignment::Left))
    .title_bottom(Line::from(bottom_right_spans).alignment(Alignment::Right))
    .borders(Borders::ALL)
    .border_type(BorderType::Rounded)
    .border_style(border_style);

  let inner = block.inner(area);
  frame.render_widget(block, area);

  let search_active = app.mode == Mode::Search;
  let has_query = !app.search_query.is_empty();

  let (search_area, list_area) = if search_active || has_query {
    let chunks = Layout::default()
      .direction(Direction::Vertical)
      .constraints([Constraint::Length(2), Constraint::Min(0)])
      .split(inner);
    (Some(chunks[0]), chunks[1])
  } else {
    (None, inner)
  };

  app.list_table_area = list_area;

  if let Some(search_area) = search_area {
    let input_area = Rect::new(search_area.x, search_area.y, search_area.width, 1);
    let divider_area = Rect::new(search_area.x, search_area.y + 1, search_area.width, 1);

    if search_active {
      let display_text = if app.search_input.is_empty() {
        Span::styled(" Search...", theme.empty_hint)
      } else {
        Span::styled(format!(" {}", app.search_input), theme.header_value)
      };
      let input = Paragraph::new(Line::from(display_text));
      frame.render_widget(input, input_area);

      let cursor_x = app.search_input.chars().count() as u16 + 1;
      frame.set_cursor_position((
        input_area.x + cursor_x.min(input_area.width.saturating_sub(1)),
        input_area.y,
      ));
    } else {
      let query_line = Paragraph::new(Line::from(vec![
        Span::styled(" /", theme.help_desc),
        Span::styled(format!(" {}", app.search_query), theme.header_value),
      ]));
      frame.render_widget(query_line, input_area);
    }

    let divider =
      Paragraph::new("─".repeat(divider_area.width as usize)).style(theme.border_unfocused);
    frame.render_widget(divider, divider_area);
  }

  if app.messages.is_empty() {
    let empty = if app.loading {
      format!(" {} Loading...", app.spinner_char())
    } else {
      "No messages".to_string()
    };
    let p = Paragraph::new(empty)
      .alignment(Alignment::Center)
      .style(theme.empty_hint);
    frame.render_widget(p, list_area);
    return;
  }

  let lines_per_item = 2u16;
  let visible_items = (list_area.height / lines_per_item) as usize;
  let has_scrollbar = app.messages.len() > visible_items;
  let scrollbar_reserve: u16 = if has_scrollbar { 1 } else { 0 };
  let accent_width = 1u16;
  let right_pad = 1u16;
  let content_width =
    list_area.width.saturating_sub(accent_width + right_pad + scrollbar_reserve) as usize;

  let items: Vec<ListItem> = app
    .messages
    .iter()
    .map(|msg| {
      let is_unread = !msg.is_read;

      let accent = if is_unread { "▎" } else { " " };
      let accent_style = if is_unread {
        theme.border_focused
      } else {
        Style::default()
      };

      let sender = parse_sender(&msg.sender);
      let date = format_date(&msg.created_at);
      let subject = msg.subject.as_deref().unwrap_or("(no subject)");
      let size = format_size(msg.size);

      let mut icons = String::new();
      if msg.is_starred {
        icons.push('★');
      }
      if msg.has_attachments {
        if !icons.is_empty() {
          icons.push(' ');
        }
        icons.push('@');
      }

      let sender_style = if is_unread {
        theme.row_unread
      } else {
        theme.row_read
      };

      let right1 = if icons.is_empty() {
        date.clone()
      } else {
        format!("{icons}  {date}")
      };
      let right1_len = display_width(&right1);
      let sender_max = content_width.saturating_sub(right1_len + 1);
      let sender_display = truncate(&sender, sender_max);

      let padding1 = content_width.saturating_sub(display_width(&sender_display) + right1_len);

      let line1 = Line::from(vec![
        Span::styled(accent, accent_style),
        Span::styled(sender_display, sender_style),
        Span::raw(" ".repeat(padding1)),
        Span::styled(right1, theme.help_desc),
      ]);

      let right2 = format!("  {size}");
      let right2_len = display_width(&right2);
      let subject_max = content_width.saturating_sub(right2_len + 1);
      let subject_display = truncate(subject, subject_max);

      let subject_style = if is_unread {
        theme.header_value
      } else {
        theme.row_read
      };

      let padding2 = content_width.saturating_sub(display_width(&subject_display) + right2_len);

      let line2 = Line::from(vec![
        Span::styled(accent, accent_style),
        Span::styled(subject_display, subject_style),
        Span::raw(" ".repeat(padding2)),
        Span::styled(right2, theme.help_desc),
      ]);

      ListItem::new(vec![line1, line2])
    })
    .collect();

  let list = List::new(items)
    .highlight_style(theme.row_selected)
    .highlight_spacing(HighlightSpacing::Always);

  frame.render_stateful_widget(list, list_area, &mut app.list_state);

  if has_scrollbar {
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
      .track_style(theme.scrollbar_track)
      .thumb_style(theme.scrollbar_thumb);

    frame.render_stateful_widget(
      scrollbar,
      list_area.inner(ratatui::layout::Margin {
        vertical: 1,
        horizontal: 0,
      }),
      &mut app.list_scrollbar_state,
    );
  }
}
