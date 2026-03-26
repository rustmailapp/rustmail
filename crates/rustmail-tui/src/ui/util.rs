pub fn display_width(s: &str) -> usize {
  unicode_width::UnicodeWidthStr::width(s)
}

pub fn format_size(bytes: i64) -> String {
  if bytes < 1024 {
    format!("{}B", bytes)
  } else if bytes < 1024 * 1024 {
    format!("{:.1}K", bytes as f64 / 1024.0)
  } else {
    format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
  }
}

pub fn format_date(iso: &str) -> String {
  iso
    .get(5..16)
    .map(|s| s.replace('T', " "))
    .unwrap_or_else(|| iso.to_string())
}

pub fn truncate(s: &str, max: usize) -> String {
  use unicode_width::UnicodeWidthStr;

  let width = UnicodeWidthStr::width(s);
  if width <= max {
    s.to_string()
  } else if max > 2 {
    let mut w = 0;
    let truncated: String = s
      .chars()
      .take_while(|c| {
        w += unicode_width::UnicodeWidthChar::width(*c).unwrap_or(0);
        w <= max - 2
      })
      .collect();
    format!("{truncated}..")
  } else {
    let mut w = 0;
    s.chars()
      .take_while(|c| {
        w += unicode_width::UnicodeWidthChar::width(*c).unwrap_or(0);
        w <= max
      })
      .collect()
  }
}

pub fn parse_sender(sender: &str) -> String {
  if let Some(start) = sender.find('<') {
    let name = sender[..start].trim().trim_matches('"');
    if name.is_empty() {
      sender
        .get(start + 1..)
        .unwrap_or(sender)
        .trim_end_matches('>')
        .to_string()
    } else {
      name.to_string()
    }
  } else {
    sender.to_string()
  }
}
