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
  let char_count = s.chars().count();
  if char_count <= max {
    s.to_string()
  } else if max > 2 {
    let truncated: String = s.chars().take(max - 2).collect();
    format!("{}..", truncated)
  } else {
    s.chars().take(max).collect()
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
