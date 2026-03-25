mod api;
mod app;
mod event;
mod ui;

pub use app::App;

use anyhow::Result;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{
  EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::prelude::*;

fn install_panic_hook() {
  let original_hook = std::panic::take_hook();
  std::panic::set_hook(Box::new(move |info| {
    let _ = disable_raw_mode();
    let _ = crossterm::execute!(std::io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
    original_hook(info);
  }));
}

pub async fn run(host: &str, port: u16) -> Result<()> {
  install_panic_hook();

  enable_raw_mode()?;
  let mut stdout = std::io::stdout();
  crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
  let backend = CrosstermBackend::new(stdout);
  let mut terminal = Terminal::new(backend)?;

  let base_url = format!("http://{}:{}", host, port);
  let ws_url = format!("ws://{}:{}/api/v1/ws", host, port);

  let mut app = App::new(base_url, ws_url);
  let result = app.run(&mut terminal).await;

  disable_raw_mode()?;
  crossterm::execute!(
    terminal.backend_mut(),
    LeaveAlternateScreen,
    DisableMouseCapture
  )?;
  terminal.show_cursor()?;

  result
}
