mod api;
mod app;
mod event;
pub mod theme;
mod ui;

pub use app::App;

use anyhow::Result;
use ratatui::crossterm::{
  event::{DisableMouseCapture, EnableMouseCapture},
  execute,
};

pub async fn run(host: &str, port: u16) -> Result<()> {
  let mut terminal = ratatui::init();
  let result = run_app(&mut terminal, host, port).await;
  ratatui::restore();
  result
}

async fn run_app(terminal: &mut ratatui::DefaultTerminal, host: &str, port: u16) -> Result<()> {
  execute!(std::io::stdout(), EnableMouseCapture)?;

  let base_url = format!("http://{}:{}", host, port);
  let ws_url = format!("ws://{}:{}/api/v1/ws", host, port);

  let mut app = App::new(base_url, ws_url);
  let result = app.run(terminal).await;

  let _ = execute!(std::io::stdout(), DisableMouseCapture);
  result
}
