use std::time::Duration;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use tokio::sync::mpsc;

use crate::api::{ApiClient, Message, MessageSummary, WsEvent};
use crate::event::{self, Event};
use crate::ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
  Normal,
  Search,
  RawView,
  Confirm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
  List,
  Preview,
}

pub struct App {
  pub running: bool,
  pub mode: Mode,
  pub focus: Focus,

  api: ApiClient,
  ws_url: String,

  pub messages: Vec<MessageSummary>,
  pub total: i64,
  pub selected: usize,
  pub offset: i64,
  pub page_size: i64,

  pub preview: Option<Message>,
  pub preview_scroll: u16,
  pub preview_loading: bool,
  last_preview_id: Option<String>,

  pub raw_content: Option<String>,
  pub raw_scroll: u16,

  pub search_query: String,
  pub search_input: String,

  pub status_message: Option<String>,
  status_ticks: u16,
  pub confirm_action: Option<String>,

  pub error: Option<String>,
  error_ticks: u16,
  pub loading: bool,
}

impl App {
  pub fn new(base_url: String, ws_url: String) -> Self {
    Self {
      running: true,
      mode: Mode::Normal,
      focus: Focus::List,

      api: ApiClient::new(base_url),
      ws_url,

      messages: Vec::new(),
      total: 0,
      selected: 0,
      offset: 0,
      page_size: 50,

      preview: None,
      preview_scroll: 0,
      preview_loading: false,
      last_preview_id: None,

      raw_content: None,
      raw_scroll: 0,

      search_query: String::new(),
      search_input: String::new(),

      status_message: None,
      status_ticks: 0,
      confirm_action: None,

      error: None,
      error_ticks: 0,
      loading: false,
    }
  }

  pub async fn run(
    &mut self,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
  ) -> Result<()> {
    let (mut events, ws_sender) = event::create_event_handler();

    self.connect_websocket(ws_sender);
    self.fetch_messages().await;

    while self.running {
      terminal.draw(|frame| ui::render(frame, self))?;

      match events.next().await? {
        Event::Key(key) => self.handle_key(key).await,
        Event::Tick => self.on_tick(),
        Event::WsMessage(msg) => self.handle_ws_message(&msg).await,
      }
    }

    Ok(())
  }

  fn connect_websocket(&self, tx: mpsc::UnboundedSender<Event>) {
    let ws_url = self.ws_url.clone();
    tokio::spawn(async move {
      loop {
        match connect_ws(&ws_url, &tx).await {
          Ok(()) | Err(_) => {}
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
      }
    });
  }

  async fn handle_key(&mut self, key: KeyEvent) {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
      self.running = false;
      return;
    }

    match self.mode {
      Mode::Search => self.handle_search_key(key).await,
      Mode::RawView => self.handle_raw_view_key(key),
      Mode::Confirm => self.handle_confirm_key(key).await,
      Mode::Normal => self.handle_normal_key(key).await,
    }
  }

  async fn handle_normal_key(&mut self, key: KeyEvent) {
    match self.focus {
      Focus::List => self.handle_list_key(key).await,
      Focus::Preview => self.handle_preview_key(key).await,
    }
  }

  async fn handle_list_key(&mut self, key: KeyEvent) {
    match key.code {
      KeyCode::Char('q') => self.running = false,
      KeyCode::Char('j') | KeyCode::Down => {
        if !self.messages.is_empty() {
          let old = self.selected;
          self.selected = (self.selected + 1).min(self.messages.len() - 1);
          if old != self.selected {
            self.load_preview().await;
          }
        }
      }
      KeyCode::Char('k') | KeyCode::Up => {
        if !self.messages.is_empty() {
          let old = self.selected;
          self.selected = self.selected.saturating_sub(1);
          if old != self.selected {
            self.load_preview().await;
          }
        }
      }
      KeyCode::Char('g') => {
        if !self.messages.is_empty() && self.selected != 0 {
          self.selected = 0;
          self.load_preview().await;
        }
      }
      KeyCode::Char('G') => {
        if !self.messages.is_empty() {
          let last = self.messages.len() - 1;
          if self.selected != last {
            self.selected = last;
            self.load_preview().await;
          }
        }
      }
      KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
        self.focus = Focus::Preview;
      }
      KeyCode::Tab => {
        self.focus = Focus::Preview;
      }
      KeyCode::Char('/') => {
        self.mode = Mode::Search;
        self.search_input = self.search_query.clone();
      }
      KeyCode::Char('r') => self.toggle_read().await,
      KeyCode::Char('s') => self.toggle_star().await,
      KeyCode::Char('d') => self.delete_selected().await,
      KeyCode::Char('D') => {
        self.mode = Mode::Confirm;
        self.confirm_action = Some("delete_all".into());
      }
      KeyCode::Char('R') => self.show_raw().await,
      KeyCode::Char(']') => self.next_page().await,
      KeyCode::Char('[') => self.prev_page().await,
      KeyCode::Char('?') => {
        self.set_status(
          "j/k:nav  Enter:preview  /:search  r:read  s:star  d:del  D:clear  R:raw  [/]:page  q:quit",
        );
      }
      _ => {}
    }
  }

  async fn handle_preview_key(&mut self, key: KeyEvent) {
    match key.code {
      KeyCode::Char('q') => self.running = false,
      KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left | KeyCode::Tab | KeyCode::BackTab => {
        self.focus = Focus::List;
      }
      KeyCode::Char('j') | KeyCode::Down => {
        self.preview_scroll = self.preview_scroll.saturating_add(1);
      }
      KeyCode::Char('k') | KeyCode::Up => {
        self.preview_scroll = self.preview_scroll.saturating_sub(1);
      }
      KeyCode::Char('r') => self.toggle_read().await,
      KeyCode::Char('s') => self.toggle_star().await,
      KeyCode::Char('d') => self.delete_selected().await,
      KeyCode::Char('R') => self.show_raw().await,
      _ => {}
    }
  }

  async fn handle_search_key(&mut self, key: KeyEvent) {
    match key.code {
      KeyCode::Enter => {
        self.search_query = self.search_input.clone();
        self.mode = Mode::Normal;
        self.offset = 0;
        self.selected = 0;
        self.fetch_messages().await;
      }
      KeyCode::Esc => {
        self.mode = Mode::Normal;
      }
      KeyCode::Backspace => {
        self.search_input.pop();
      }
      KeyCode::Char(c) => {
        self.search_input.push(c);
      }
      _ => {}
    }
  }

  fn handle_raw_view_key(&mut self, key: KeyEvent) {
    match key.code {
      KeyCode::Char('q') | KeyCode::Esc => {
        self.mode = Mode::Normal;
        self.raw_content = None;
        self.raw_scroll = 0;
      }
      KeyCode::Char('j') | KeyCode::Down => {
        self.raw_scroll = self.raw_scroll.saturating_add(1);
      }
      KeyCode::Char('k') | KeyCode::Up => {
        self.raw_scroll = self.raw_scroll.saturating_sub(1);
      }
      _ => {}
    }
  }

  async fn handle_confirm_key(&mut self, key: KeyEvent) {
    match key.code {
      KeyCode::Char('y') | KeyCode::Char('Y') => {
        if self.confirm_action.as_deref() == Some("delete_all") {
          self.delete_all().await;
        }
        self.mode = Mode::Normal;
        self.confirm_action = None;
      }
      _ => {
        self.mode = Mode::Normal;
        self.confirm_action = None;
      }
    }
  }

  fn on_tick(&mut self) {
    if self.status_message.is_some() {
      self.status_ticks += 1;
      if self.status_ticks >= 30 {
        self.status_message = None;
        self.status_ticks = 0;
      }
    }
    if self.error.is_some() {
      self.error_ticks += 1;
      if self.error_ticks >= 50 {
        self.error = None;
        self.error_ticks = 0;
      }
    }
  }

  fn set_status(&mut self, msg: &str) {
    self.status_message = Some(msg.into());
    self.status_ticks = 0;
  }

  fn set_error(&mut self, msg: String) {
    self.error = Some(msg);
    self.error_ticks = 0;
  }

  pub async fn fetch_messages(&mut self) {
    self.loading = true;
    let query = if self.search_query.is_empty() {
      None
    } else {
      Some(self.search_query.as_str())
    };

    match self
      .api
      .list_messages(query, self.page_size, self.offset)
      .await
    {
      Ok(resp) => {
        self.messages = resp.messages;
        self.total = resp.total;
        self.error = None;
        self.error_ticks = 0;
        if self.selected >= self.messages.len() && !self.messages.is_empty() {
          self.selected = self.messages.len() - 1;
        }
        self.load_preview().await;
      }
      Err(e) => {
        self.set_error(format!("Failed to fetch messages: {}", e));
      }
    }
    self.loading = false;
  }

  async fn load_preview(&mut self) {
    let Some(msg) = self.messages.get(self.selected) else {
      self.preview = None;
      self.last_preview_id = None;
      return;
    };

    if self.last_preview_id.as_deref() == Some(&msg.id) {
      return;
    }

    let target_id = msg.id.clone();
    let was_unread = !msg.is_read;
    self.preview_loading = true;
    self.preview_scroll = 0;

    match self.api.get_message(&target_id).await {
      Ok(detail) => {
        self.last_preview_id = Some(target_id.clone());
        self.preview = Some(detail);

        if was_unread {
          let _ = self.api.update_message(&target_id, Some(true), None).await;
          if let Some(m) = self.messages.iter_mut().find(|m| m.id == target_id) {
            m.is_read = true;
          }
        }
      }
      Err(e) => {
        self.set_error(format!("Failed to load message: {}", e));
      }
    }
    self.preview_loading = false;
  }

  async fn toggle_read(&mut self) {
    let Some(msg) = self.messages.get(self.selected) else {
      return;
    };
    let new_state = !msg.is_read;
    let id = msg.id.clone();
    if self
      .api
      .update_message(&id, Some(new_state), None)
      .await
      .is_ok()
      && let Some(m) = self.messages.iter_mut().find(|m| m.id == id)
    {
      m.is_read = new_state;
    }
  }

  async fn toggle_star(&mut self) {
    let Some(msg) = self.messages.get(self.selected) else {
      return;
    };
    let new_state = !msg.is_starred;
    let id = msg.id.clone();
    if self
      .api
      .update_message(&id, None, Some(new_state))
      .await
      .is_ok()
      && let Some(m) = self.messages.iter_mut().find(|m| m.id == id)
    {
      m.is_starred = new_state;
    }
  }

  async fn delete_selected(&mut self) {
    let Some(msg) = self.messages.get(self.selected) else {
      return;
    };
    let id = msg.id.clone();
    if self.api.delete_message(&id).await.is_ok() {
      self.messages.retain(|m| m.id != id);
      self.total -= 1;
      if self.selected >= self.messages.len() && self.selected > 0 {
        self.selected -= 1;
      }
      self.last_preview_id = None;
      self.load_preview().await;
    }
  }

  async fn delete_all(&mut self) {
    if self.api.delete_all_messages().await.is_ok() {
      self.messages.clear();
      self.total = 0;
      self.selected = 0;
      self.preview = None;
      self.last_preview_id = None;
    }
  }

  async fn show_raw(&mut self) {
    let Some(msg) = self.messages.get(self.selected) else {
      return;
    };
    let id = msg.id.clone();
    match self.api.get_raw_message(&id).await {
      Ok(raw) => {
        self.raw_content = Some(raw);
        self.raw_scroll = 0;
        self.mode = Mode::RawView;
      }
      Err(e) => {
        self.set_error(format!("Failed to load raw message: {}", e));
      }
    }
  }

  async fn next_page(&mut self) {
    let new_offset = self.offset + self.page_size;
    if new_offset < self.total {
      self.offset = new_offset;
      self.selected = 0;
      self.last_preview_id = None;
      self.fetch_messages().await;
    }
  }

  async fn prev_page(&mut self) {
    if self.offset > 0 {
      self.offset = (self.offset - self.page_size).max(0);
      self.selected = 0;
      self.last_preview_id = None;
      self.fetch_messages().await;
    }
  }

  async fn handle_ws_message(&mut self, msg: &str) {
    let Ok(event) = serde_json::from_str::<WsEvent>(msg) else {
      return;
    };

    match event {
      WsEvent::MessageNew(summary) => {
        self.total += 1;
        if self.offset == 0 {
          self.messages.insert(0, summary);
          if self.messages.len() > self.page_size as usize {
            self.messages.pop();
          }
          self.selected += 1;
        }
      }
      WsEvent::MessageDelete { id } => {
        if let Some(pos) = self.messages.iter().position(|m| m.id == id) {
          self.messages.remove(pos);
          self.total -= 1;
          if self.selected >= self.messages.len() && self.selected > 0 {
            self.selected -= 1;
          }
          if self.last_preview_id.as_deref() == Some(&id) {
            self.last_preview_id = None;
            self.load_preview().await;
          }
        } else {
          self.total -= 1;
        }
      }
      WsEvent::MessageRead { id, is_read } => {
        if let Some(msg) = self.messages.iter_mut().find(|m| m.id == id) {
          msg.is_read = is_read;
        }
      }
      WsEvent::MessageStarred { id, is_starred } => {
        if let Some(msg) = self.messages.iter_mut().find(|m| m.id == id) {
          msg.is_starred = is_starred;
        }
      }
      WsEvent::MessageTags { id, tags } => {
        if let Some(msg) = self.messages.iter_mut().find(|m| m.id == id) {
          msg.tags = tags;
        }
      }
      WsEvent::MessagesClear => {
        self.messages.clear();
        self.total = 0;
        self.selected = 0;
        self.preview = None;
        self.last_preview_id = None;
      }
    }
  }

  pub fn current_page(&self) -> i64 {
    self.offset / self.page_size + 1
  }

  pub fn total_pages(&self) -> i64 {
    ((self.total as f64) / (self.page_size as f64)).ceil() as i64
  }

  pub fn unread_count(&self) -> usize {
    self.messages.iter().filter(|m| !m.is_read).count()
  }
}

async fn connect_ws(url: &str, tx: &mpsc::UnboundedSender<Event>) -> Result<()> {
  use futures_util::StreamExt;
  use tokio_tungstenite::connect_async;

  let (ws_stream, _) = connect_async(url).await?;
  let (_, mut read) = ws_stream.split();

  while let Some(msg) = read.next().await {
    match msg {
      Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
        let _ = tx.send(Event::WsMessage(text.to_string()));
      }
      Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => break,
      Err(_) => break,
      _ => {}
    }
  }

  Ok(())
}
