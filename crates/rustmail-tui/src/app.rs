use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
  KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::prelude::*;
use ratatui::widgets::{ListState, ScrollbarState};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::api::{
  ApiClient, ListResponse, Message, MessageSummary, RAW_PREVIEW_LIMIT_BYTES, WsEvent,
};
use crate::event::{self, Event, RawTarget};
use crate::ui;
use crate::ui::util::format_size;

const STALE_VIEW_REFETCH_INTERVAL: Duration = Duration::from_secs(2);
/// Upper bound on WebSocket deltas remembered while a list fetch is in flight.
/// Past it the buffer is dropped and the landed snapshot is marked stale, so a
/// burst costs one extra resync instead of unbounded memory.
const PENDING_DELTA_CAPACITY: usize = 256;
const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
  Normal,
  Search,
  RawView,
  Confirm,
  Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
  List,
  Preview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewTab {
  Text,
  Headers,
  Raw,
}

/// WebSocket deltas that arrived while a list fetch was in flight, replayed
/// onto its snapshot when it lands so they are not lost or undone.
#[derive(Default)]
struct PendingDeltas {
  events: Vec<WsEvent>,
  overflowed: bool,
}

impl PendingDeltas {
  fn record(&mut self, delta: WsEvent) {
    if self.overflowed {
      return;
    }
    if self.events.len() >= PENDING_DELTA_CAPACITY {
      self.events.clear();
      self.overflowed = true;
      return;
    }
    self.events.push(delta);
  }
}

/// The list order and cursor captured before the list mutates, so the cursor
/// can be put back on the same message once it has.
struct SelectionAnchor {
  ids: Vec<String>,
  index: usize,
}

pub struct App {
  pub running: bool,
  pub mode: Mode,
  pub focus: Focus,

  api: ApiClient,
  ws_url: String,
  event_tx: mpsc::Sender<Event>,
  tasks: JoinSet<()>,
  fetch_generation: u64,
  last_request_id: u64,
  pending_deltas: PendingDeltas,

  pub messages: Vec<MessageSummary>,
  pub total: i64,
  pub selected: usize,
  pub offset: i64,
  pub page_size: i64,

  pub list_state: ListState,
  pub list_scrollbar_state: ScrollbarState,

  pub preview: Option<Message>,
  pub preview_scroll: u16,
  pub preview_loading: bool,
  pub preview_scrollbar_state: ScrollbarState,
  pub preview_tab: PreviewTab,
  pub preview_raw: Option<String>,
  pub preview_raw_notice: Option<String>,
  last_preview_id: Option<String>,
  pending_preview: Option<u64>,

  pub raw_content: Option<String>,
  pub raw_notice: Option<String>,
  pub raw_scroll: u16,
  pending_raw: Option<(RawTarget, u64)>,

  pub search_query: String,
  pub search_input: String,

  pub confirm_action: Option<String>,

  pub error: Option<String>,
  error_ticks: u16,
  pub loading: bool,
  view_stale: bool,
  view_changed: bool,
  last_fetch_at: Option<Instant>,

  pub ws_connected: bool,
  ws_ever_connected: bool,
  pub spinner_frame: usize,

  pub list_area: Rect,
  pub list_table_area: Rect,
  pub preview_area: Rect,
  pub tab_area: Rect,
  pub tab_ranges: [(u16, u16); 3],
}

impl App {
  pub fn new(base_url: String, ws_url: String, event_tx: mpsc::Sender<Event>) -> Self {
    Self {
      running: true,
      mode: Mode::Normal,
      focus: Focus::List,

      api: ApiClient::new(base_url),
      ws_url,
      event_tx,
      tasks: JoinSet::new(),
      fetch_generation: 0,
      last_request_id: 0,
      pending_deltas: PendingDeltas::default(),

      messages: Vec::new(),
      total: 0,
      selected: 0,
      offset: 0,
      page_size: 50,

      list_state: ListState::default().with_selected(Some(0)),
      list_scrollbar_state: ScrollbarState::default(),

      preview: None,
      preview_scroll: 0,
      preview_loading: false,
      preview_scrollbar_state: ScrollbarState::default(),
      preview_tab: PreviewTab::Text,
      preview_raw: None,
      preview_raw_notice: None,
      last_preview_id: None,
      pending_preview: None,

      raw_content: None,
      raw_notice: None,
      raw_scroll: 0,
      pending_raw: None,

      search_query: String::new(),
      search_input: String::new(),

      confirm_action: None,

      error: None,
      error_ticks: 0,
      loading: false,
      view_stale: false,
      view_changed: false,
      last_fetch_at: None,

      ws_connected: false,
      ws_ever_connected: false,
      spinner_frame: 0,

      list_area: Rect::default(),
      list_table_area: Rect::default(),
      preview_area: Rect::default(),
      tab_area: Rect::default(),
      tab_ranges: [(0, 0); 3],
    }
  }

  pub async fn run(
    &mut self,
    terminal: &mut DefaultTerminal,
    events: &mut event::EventHandler,
  ) -> Result<()> {
    let result = self.event_loop(terminal, events).await;
    self.shutdown_tasks().await;
    result
  }

  async fn event_loop(
    &mut self,
    terminal: &mut DefaultTerminal,
    events: &mut event::EventHandler,
  ) -> Result<()> {
    self.connect_websocket();
    self.fetch_messages().await;

    while self.running {
      let first = events.next().await?;
      self.dispatch(first).await;

      while self.running {
        let Some(event) = events.try_next() else {
          break;
        };
        self.dispatch(event).await;
      }

      if !self.running {
        break;
      }
      terminal.draw(|frame| ui::render(frame, self))?;
    }

    Ok(())
  }

  async fn dispatch(&mut self, event: Event) {
    match event {
      Event::Key(key) => self.handle_key(key).await,
      Event::Mouse(mouse) => self.handle_mouse(mouse).await,
      Event::Resize => {}
      Event::Tick => {
        self.on_tick();
        self.refetch_if_stale().await;
      }
      Event::WsMessage(msg) => self.handle_ws_message(&msg).await,
      Event::WsStatus(connected) => self.handle_ws_status(connected).await,
      Event::WsOverflow => self.handle_ws_overflow(),
      Event::MessagesFetched { generation, result } => {
        self.handle_messages_fetched(generation, result).await
      }
      Event::PreviewLoaded {
        request,
        id,
        was_unread,
        result,
      } => {
        self
          .handle_preview_loaded(request, id, was_unread, result)
          .await
      }
      Event::RawLoaded {
        target,
        request,
        id,
        size,
        result,
      } => {
        self
          .handle_raw_loaded(target, request, id, size, result)
          .await
      }
      Event::Patched {
        id,
        is_read,
        is_starred,
        result,
      } => self.handle_patched(id, is_read, is_starred, result),
      Event::Deleted { id, result } => self.handle_deleted(id, result).await,
      Event::AllDeleted { result } => self.handle_all_deleted(result).await,
    }
  }

  fn next_request_id(&mut self) -> u64 {
    self.last_request_id += 1;
    self.last_request_id
  }

  /// Aborts every background task this app owns and waits for them to stop.
  async fn shutdown_tasks(&mut self) {
    self.tasks.shutdown().await;
  }

  fn reap_finished_tasks(&mut self) {
    while let Some(outcome) = self.tasks.try_join_next() {
      if let Err(e) = outcome
        && e.is_panic()
      {
        self.set_error(format!("Background task failed: {}", e));
      }
    }
  }

  fn spawn_owned<F>(&mut self, fut: F)
  where
    F: std::future::Future<Output = ()> + Send + 'static,
  {
    self.reap_finished_tasks();
    self.tasks.spawn(fut);
  }

  fn spawn_and_send<F>(&mut self, fut: F)
  where
    F: std::future::Future<Output = Event> + Send + 'static,
  {
    let tx = self.event_tx.clone();
    self.spawn_owned(async move {
      let event = fut.await;
      let _ = tx.send(event).await;
    });
  }

  fn connect_websocket(&mut self) {
    let ws_url = self.ws_url.clone();
    let tx = self.event_tx.clone();
    self.spawn_owned(async move {
      let mut delay = Duration::from_secs(2);
      loop {
        let _ = tx.send(Event::WsStatus(false)).await;
        if connect_ws(&ws_url, &tx).await.is_ok() {
          delay = Duration::from_secs(2);
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(30));
      }
    });
  }

  fn handle_ws_overflow(&mut self) {
    self.view_stale = true;
    if self.loading {
      self.pending_deltas.overflowed = true;
    }
  }

  async fn handle_ws_status(&mut self, connected: bool) {
    let reconnected = connected && !self.ws_connected && self.ws_ever_connected;
    self.ws_connected = connected;
    self.ws_ever_connected |= connected;
    if reconnected {
      self.fetch_messages().await;
    }
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
      Mode::Help => self.handle_help_key(key),
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
      KeyCode::Char('j') | KeyCode::Down if self.select_next() => {
        self.load_preview().await;
      }
      KeyCode::Char('k') | KeyCode::Up if self.select_prev() => {
        self.load_preview().await;
      }
      KeyCode::Char('g') if self.select_message(0) => {
        self.load_preview().await;
      }
      KeyCode::Char('G') => {
        let last = self.messages.len().saturating_sub(1);
        if self.select_message(last) {
          self.load_preview().await;
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
        self.mode = Mode::Help;
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
      KeyCode::Char('1') => self.switch_preview_tab(PreviewTab::Text),
      KeyCode::Char('2') => self.switch_preview_tab(PreviewTab::Headers),
      KeyCode::Char('3') => {
        self.switch_preview_tab(PreviewTab::Raw);
        self.ensure_raw_loaded().await;
      }
      KeyCode::Char('r') => self.toggle_read().await,
      KeyCode::Char('s') => self.toggle_star().await,
      KeyCode::Char('d') => self.delete_selected().await,
      KeyCode::Char('R') => self.show_raw().await,
      _ => {}
    }
  }

  fn switch_preview_tab(&mut self, tab: PreviewTab) {
    self.preview_tab = tab;
    self.preview_scroll = 0;
  }

  async fn ensure_raw_loaded(&mut self) {
    if self.preview_raw.is_some() {
      return;
    }
    let Some(msg) = self.selected_message() else {
      return;
    };
    let id = msg.id.clone();
    let size = msg.size;
    let request = self.next_request_id();
    self.pending_raw = Some((RawTarget::Preview, request));

    let api = self.api.clone();
    self.spawn_and_send(async move {
      let result = api
        .get_raw_message(&id, RAW_PREVIEW_LIMIT_BYTES)
        .await
        .map_err(|e| e.to_string());
      Event::RawLoaded {
        target: RawTarget::Preview,
        request,
        id,
        size,
        result,
      }
    });
  }

  async fn handle_search_key(&mut self, key: KeyEvent) {
    match key.code {
      KeyCode::Enter => {
        self.search_query = self.search_input.clone();
        self.mode = Mode::Normal;
        self.offset = 0;
        self.view_changed = true;
        self.fetch_messages().await;
      }
      KeyCode::Esc => {
        if self.search_input.is_empty() && !self.search_query.is_empty() {
          self.search_query.clear();
          self.mode = Mode::Normal;
          self.offset = 0;
          self.view_changed = true;
          self.fetch_messages().await;
        } else {
          self.mode = Mode::Normal;
        }
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
        self.raw_notice = None;
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

  fn handle_help_key(&mut self, key: KeyEvent) {
    match key.code {
      KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::Enter => {
        self.mode = Mode::Normal;
      }
      _ => {}
    }
  }

  async fn handle_mouse(&mut self, event: MouseEvent) {
    match event.kind {
      MouseEventKind::Down(MouseButton::Left) => {
        let pos = Position::new(event.column, event.row);

        if self.tab_area.contains(pos) {
          let abs_x = event.column;
          let tabs = [PreviewTab::Text, PreviewTab::Headers, PreviewTab::Raw];
          for (i, tab) in tabs.iter().enumerate() {
            let (start, end) = self.tab_ranges[i];
            let tab_start = self.tab_area.x + start;
            let tab_end = self.tab_area.x + end;
            if abs_x >= tab_start && abs_x < tab_end {
              if *tab == PreviewTab::Raw {
                self.switch_preview_tab(PreviewTab::Raw);
                self.ensure_raw_loaded().await;
              } else {
                self.switch_preview_tab(*tab);
              }
              break;
            }
          }
          self.focus = Focus::Preview;
        } else if self.list_table_area.contains(pos) {
          self.focus = Focus::List;
          let row_offset = event.row.saturating_sub(self.list_table_area.y);
          let scroll_offset = self.list_state.offset();
          let idx = scroll_offset + (row_offset / 2) as usize;
          if idx < self.messages.len() && idx != self.selected {
            self.selected = idx;
            self.sync_list_state();
            self.load_preview().await;
          }
        } else if self.list_area.contains(pos) {
          self.focus = Focus::List;
        } else if self.preview_area.contains(pos) {
          self.focus = Focus::Preview;
        }
      }
      MouseEventKind::ScrollDown => {
        let pos = Position::new(event.column, event.row);
        if self.list_area.contains(pos) {
          if self.select_next() {
            self.load_preview().await;
          }
        } else if self.preview_area.contains(pos) {
          self.preview_scroll = self.preview_scroll.saturating_add(3);
        }
      }
      MouseEventKind::ScrollUp => {
        let pos = Position::new(event.column, event.row);
        if self.list_area.contains(pos) {
          if self.select_prev() {
            self.load_preview().await;
          }
        } else if self.preview_area.contains(pos) {
          self.preview_scroll = self.preview_scroll.saturating_sub(3);
        }
      }
      _ => {}
    }
  }

  fn on_tick(&mut self) {
    if self.error.is_some() {
      self.error_ticks += 1;
      if self.error_ticks >= 50 {
        self.error = None;
        self.error_ticks = 0;
      }
    }
    if self.loading || self.preview_loading {
      self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES.len();
    }
  }

  async fn refetch_if_stale(&mut self) {
    let throttle_elapsed = self
      .last_fetch_at
      .is_none_or(|at| at.elapsed() >= STALE_VIEW_REFETCH_INTERVAL);
    if self.view_stale && throttle_elapsed {
      self.fetch_messages().await;
    }
  }

  fn set_error(&mut self, msg: String) {
    self.error = Some(msg);
    self.error_ticks = 0;
  }

  fn select_message(&mut self, idx: usize) -> bool {
    if idx < self.messages.len() && idx != self.selected {
      self.selected = idx;
      self.sync_list_state();
      true
    } else {
      false
    }
  }

  fn select_next(&mut self) -> bool {
    if self.messages.is_empty() {
      return false;
    }
    let idx = (self.selected + 1).min(self.messages.len() - 1);
    self.select_message(idx)
  }

  fn select_prev(&mut self) -> bool {
    if self.messages.is_empty() {
      return false;
    }
    let idx = self.selected.saturating_sub(1);
    self.select_message(idx)
  }

  fn selected_message(&self) -> Option<&MessageSummary> {
    self.messages.get(self.selected)
  }

  fn capture_selection(&self) -> SelectionAnchor {
    SelectionAnchor {
      ids: self.messages.iter().map(|m| m.id.clone()).collect(),
      index: self.selected,
    }
  }

  /// An anchor that keeps the selected message if the new list still holds
  /// it and otherwise lands on the top row, for when the view itself changed.
  fn capture_selected_only(&self) -> SelectionAnchor {
    SelectionAnchor {
      ids: self
        .selected_message()
        .map(|m| m.id.clone())
        .into_iter()
        .collect(),
      index: 0,
    }
  }

  /// Re-resolves the cursor against the mutated list. Returns whether it now
  /// rests on a different message.
  fn reanchor_selection(&mut self, anchor: SelectionAnchor) -> bool {
    let previous = anchor.ids.get(anchor.index);
    self.selected = resolve_selection(&anchor.ids, anchor.index, &self.messages);
    self.sync_list_state();
    self.selected_message().map(|m| &m.id) != previous
  }

  /// Re-resolves the cursor and, when it moved to a different message, drops
  /// the old preview and loads the new one. While a list fetch is in flight
  /// the preview is left for the landing snapshot to settle, since the list
  /// it would be read against is about to be replaced.
  async fn restore_selection(&mut self, anchor: SelectionAnchor) {
    if self.reanchor_selection(anchor) && !self.loading {
      self.retarget_preview().await;
    }
  }

  fn preview_matches_selection(&self) -> bool {
    self.last_preview_id.as_ref() == self.selected_message().map(|m| &m.id)
  }

  async fn retarget_preview(&mut self) {
    self.preview = None;
    self.last_preview_id = None;
    self.pending_preview = None;
    self.preview_raw = None;
    self.preview_raw_notice = None;
    self.pending_raw = None;
    self.load_preview().await;
  }

  pub fn sync_list_state(&mut self) {
    self.list_state.select(Some(self.selected));
    self.list_scrollbar_state = self
      .list_scrollbar_state
      .content_length(self.messages.len())
      .position(self.selected);
  }

  pub async fn fetch_messages(&mut self) {
    self.loading = true;
    self.last_fetch_at = Some(Instant::now());
    self.fetch_generation += 1;
    let generation = self.fetch_generation;

    let api = self.api.clone();
    let query = self.search_query.clone();
    let page_size = self.page_size;
    let offset = self.offset;

    self.spawn_and_send(async move {
      let q = if query.is_empty() {
        None
      } else {
        Some(query.as_str())
      };
      let result = api
        .list_messages(q, page_size, offset)
        .await
        .map_err(|e| e.to_string());
      Event::MessagesFetched { generation, result }
    });
  }

  async fn handle_messages_fetched(
    &mut self,
    generation: u64,
    result: Result<ListResponse, String>,
  ) {
    if generation != self.fetch_generation {
      return;
    }
    self.loading = false;
    let pending = std::mem::take(&mut self.pending_deltas);
    let view_changed = std::mem::take(&mut self.view_changed);
    match result {
      Ok(resp) => {
        let anchor = if view_changed {
          self.capture_selected_only()
        } else {
          self.capture_selection()
        };
        self.messages = resp.messages;
        self.total = resp.total;
        self.view_stale = pending.overflowed;
        for delta in pending.events {
          self.replay_delta(delta);
        }
        self.error = None;
        self.error_ticks = 0;
        let moved = self.reanchor_selection(anchor);
        if moved || !self.preview_matches_selection() {
          self.retarget_preview().await;
        }
      }
      Err(e) => {
        self.set_error(format!("Failed to fetch messages: {}", e));
      }
    }
  }

  async fn load_preview(&mut self) {
    let Some(msg) = self.selected_message() else {
      self.preview = None;
      self.last_preview_id = None;
      self.pending_preview = None;
      self.preview_raw = None;
      self.preview_raw_notice = None;
      self.pending_raw = None;
      return;
    };

    if self.last_preview_id.as_deref() == Some(&msg.id) {
      return;
    }

    let target_id = msg.id.clone();
    let was_unread = !msg.is_read;
    self.preview_loading = true;
    self.preview_scroll = 0;
    self.preview_raw = None;
    self.preview_raw_notice = None;
    self.preview_tab = PreviewTab::Text;
    let request = self.next_request_id();
    self.pending_preview = Some(request);
    self.pending_raw = None;

    let api = self.api.clone();
    self.spawn_and_send(async move {
      let result = api.get_message(&target_id).await.map_err(|e| e.to_string());
      Event::PreviewLoaded {
        request,
        id: target_id,
        was_unread,
        result,
      }
    });
  }

  async fn handle_preview_loaded(
    &mut self,
    request: u64,
    id: String,
    was_unread: bool,
    result: Result<Message, String>,
  ) {
    if self.pending_preview != Some(request) {
      return;
    }
    self.pending_preview = None;
    self.preview_loading = false;

    match result {
      Ok(detail) => {
        self.last_preview_id = Some(id.clone());
        self.preview = Some(detail);
        if was_unread {
          self.spawn_patch(id, Some(true), None);
        }
      }
      Err(e) => {
        self.set_error(format!("Failed to load message: {}", e));
      }
    }
  }

  fn spawn_patch(&mut self, id: String, is_read: Option<bool>, is_starred: Option<bool>) {
    let api = self.api.clone();
    let patch_id = id.clone();
    self.spawn_and_send(async move {
      let result = api
        .update_message(&patch_id, is_read, is_starred)
        .await
        .map_err(|e| e.to_string());
      Event::Patched {
        id: patch_id,
        is_read,
        is_starred,
        result,
      }
    });
  }

  async fn toggle_read(&mut self) {
    let Some(msg) = self.selected_message() else {
      return;
    };
    let new_state = !msg.is_read;
    let id = msg.id.clone();
    self.spawn_patch(id, Some(new_state), None);
  }

  async fn toggle_star(&mut self) {
    let Some(msg) = self.selected_message() else {
      return;
    };
    let new_state = !msg.is_starred;
    let id = msg.id.clone();
    self.spawn_patch(id, None, Some(new_state));
  }

  fn handle_patched(
    &mut self,
    id: String,
    is_read: Option<bool>,
    is_starred: Option<bool>,
    result: Result<(), String>,
  ) {
    if result.is_err() {
      return;
    }
    let Some(m) = self.messages.iter_mut().find(|m| m.id == id) else {
      return;
    };
    if let Some(v) = is_read {
      m.is_read = v;
    }
    if let Some(v) = is_starred {
      m.is_starred = v;
    }
  }

  async fn delete_selected(&mut self) {
    let Some(msg) = self.selected_message() else {
      return;
    };
    let id = msg.id.clone();
    let api = self.api.clone();
    self.spawn_and_send(async move {
      let result = api.delete_message(&id).await.map_err(|e| e.to_string());
      Event::Deleted { id, result }
    });
  }

  async fn handle_deleted(&mut self, id: String, result: Result<(), String>) {
    if result.is_err() {
      return;
    }
    let anchor = self.capture_selection();
    self.messages.retain(|m| m.id != id);
    self.restore_selection(anchor).await;
  }

  async fn delete_all(&mut self) {
    let api = self.api.clone();
    self.spawn_and_send(async move {
      let result = api.delete_all_messages().await.map_err(|e| e.to_string());
      Event::AllDeleted { result }
    });
  }

  async fn handle_all_deleted(&mut self, result: Result<(), String>) {
    if result.is_err() {
      return;
    }
    let anchor = self.capture_selection();
    self.messages.clear();
    self.restore_selection(anchor).await;
  }

  async fn show_raw(&mut self) {
    let Some(msg) = self.selected_message() else {
      return;
    };
    let id = msg.id.clone();
    let size = msg.size;
    let request = self.next_request_id();
    self.pending_raw = Some((RawTarget::FullView, request));

    let api = self.api.clone();
    self.spawn_and_send(async move {
      let result = api
        .get_raw_message(&id, RAW_PREVIEW_LIMIT_BYTES)
        .await
        .map_err(|e| e.to_string());
      Event::RawLoaded {
        target: RawTarget::FullView,
        request,
        id,
        size,
        result,
      }
    });
  }

  async fn handle_raw_loaded(
    &mut self,
    target: RawTarget,
    request: u64,
    id: String,
    size: i64,
    result: Result<String, String>,
  ) {
    if self.pending_raw != Some((target, request)) {
      return;
    }
    self.pending_raw = None;

    match result {
      Ok(raw) => {
        let notice = raw_truncation_notice(size, &self.api.export_url(&id));
        match target {
          RawTarget::FullView => {
            self.raw_content = Some(raw);
            self.raw_notice = notice;
            self.raw_scroll = 0;
            self.mode = Mode::RawView;
          }
          RawTarget::Preview => {
            self.preview_raw = Some(raw);
            self.preview_raw_notice = notice;
          }
        }
      }
      Err(e) => {
        let message = match target {
          RawTarget::FullView => format!("Failed to load raw message: {}", e),
          RawTarget::Preview => format!("Failed to load raw: {}", e),
        };
        self.set_error(message);
      }
    }
  }

  async fn next_page(&mut self) {
    let new_offset = self.offset + self.page_size;
    if new_offset < self.total {
      self.offset = new_offset;
      self.view_changed = true;
      self.fetch_messages().await;
    }
  }

  async fn prev_page(&mut self) {
    if self.offset > 0 {
      self.offset = (self.offset - self.page_size).max(0);
      self.view_changed = true;
      self.fetch_messages().await;
    }
  }

  async fn handle_ws_message(&mut self, msg: &str) {
    let Ok(event) = serde_json::from_str::<WsEvent>(msg) else {
      return;
    };

    if self.loading {
      self.pending_deltas.record(event.clone());
    }

    match event {
      WsEvent::MessageNew(_) if !self.search_query.is_empty() => {
        self.view_stale = true;
      }
      WsEvent::MessageNew(summary) => {
        self.total += 1;
        if self.offset == 0 {
          let anchor = self.capture_selection();
          self.messages.insert(0, summary);
          self.messages.truncate(self.page_size as usize);
          self.restore_selection(anchor).await;
        }
      }
      WsEvent::MessageDelete { id } => {
        let position = self.messages.iter().position(|m| m.id == id);
        if position.is_some() || self.search_query.is_empty() {
          self.total = (self.total - 1).max(0);
        } else {
          self.view_stale = true;
        }
        if let Some(pos) = position {
          let anchor = self.capture_selection();
          self.messages.remove(pos);
          self.restore_selection(anchor).await;
        }
      }
      update @ (WsEvent::MessageRead { .. }
      | WsEvent::MessageStarred { .. }
      | WsEvent::MessageTags { .. }) => self.apply_update(update),
      WsEvent::MessagesClear => {
        let anchor = self.capture_selection();
        self.messages.clear();
        self.total = 0;
        self.restore_selection(anchor).await;
      }
    }
  }

  fn apply_update(&mut self, update: WsEvent) {
    match update {
      WsEvent::MessageRead { id, is_read } => {
        if let Some(msg) = self.message_mut(&id) {
          msg.is_read = is_read;
        }
      }
      WsEvent::MessageStarred { id, is_starred } => {
        if let Some(msg) = self.message_mut(&id) {
          msg.is_starred = is_starred;
        }
      }
      WsEvent::MessageTags { id, tags } => {
        if let Some(msg) = self.message_mut(&id) {
          msg.tags = tags;
        }
      }
      WsEvent::MessageNew(_) | WsEvent::MessageDelete { .. } | WsEvent::MessagesClear => {}
    }
  }

  fn message_mut(&mut self, id: &str) -> Option<&mut MessageSummary> {
    self.messages.iter_mut().find(|m| m.id == id)
  }

  /// Re-applies a delta recorded during an in-flight fetch onto the snapshot
  /// that fetch returned. Selection is left alone: the caller re-resolves it
  /// by message id once every delta has been replayed. A delta whose effect
  /// on this page cannot be decided locally marks the view stale instead of
  /// guessing.
  fn replay_delta(&mut self, delta: WsEvent) {
    match delta {
      WsEvent::MessageNew(summary) => {
        if self.messages.iter().any(|m| m.id == summary.id) {
          return;
        }
        if !self.search_query.is_empty() || self.offset != 0 {
          self.view_stale = true;
          return;
        }
        self.total += 1;
        self.messages.insert(0, summary);
        self.messages.truncate(self.page_size as usize);
      }
      WsEvent::MessageDelete { id } => match self.messages.iter().position(|m| m.id == id) {
        Some(pos) => {
          self.messages.remove(pos);
          self.total = (self.total - 1).max(0);
        }
        None => self.view_stale = true,
      },
      WsEvent::MessagesClear => {
        self.messages.clear();
        self.total = 0;
      }
      update => self.apply_update(update),
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

  pub fn spinner_char(&self) -> char {
    SPINNER_FRAMES[self.spinner_frame]
  }
}

/// Returns the banner shown above a raw preview that was cut at
/// [`RAW_PREVIEW_LIMIT_BYTES`], or `None` when the whole message fits.
fn raw_truncation_notice(size: i64, export_url: &str) -> Option<String> {
  (size > RAW_PREVIEW_LIMIT_BYTES).then(|| {
    format!(
      "Showing the first {} of {}. Full source: {}",
      format_size(RAW_PREVIEW_LIMIT_BYTES),
      format_size(size),
      export_url
    )
  })
}

/// Returns the index in `new` of the message that was selected at `index` in
/// `old`. When that message is gone the cursor goes to the first message that
/// followed it and survived, which is the row that took its place, then to
/// the nearest surviving one above it, and finally to `index` clamped to the
/// new list.
fn resolve_selection(old: &[String], index: usize, new: &[MessageSummary]) -> usize {
  let positions: HashMap<&str, usize> = new
    .iter()
    .enumerate()
    .map(|(pos, m)| (m.id.as_str(), pos))
    .collect();
  let position_of = |id: &String| positions.get(id.as_str()).copied();
  let (above, from_anchor) = old.split_at(index.min(old.len()));
  from_anchor
    .iter()
    .find_map(position_of)
    .or_else(|| above.iter().rev().find_map(position_of))
    .unwrap_or_else(|| index.min(new.len().saturating_sub(1)))
}

async fn connect_ws(url: &str, tx: &mpsc::Sender<Event>) -> Result<()> {
  use futures_util::StreamExt;
  use tokio_tungstenite::connect_async;

  let (ws_stream, _) = connect_async(url).await?;
  let _ = tx.send(Event::WsStatus(true)).await;
  let (_, mut read) = ws_stream.split();

  while let Some(msg) = read.next().await {
    match msg {
      Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
        forward_ws_frame(tx, text.to_string());
      }
      Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => break,
      Err(_) => break,
      _ => {}
    }
  }

  Ok(())
}

/// Forwards a WebSocket frame without blocking the socket reader. If the
/// bounded event queue is full, the frame is dropped and a single
/// [`Event::WsOverflow`] marker is attempted in its place, so the app marks
/// its view stale and resyncs once it catches up, instead of the reader
/// stalling and the server treating this client as lagging.
fn forward_ws_frame(tx: &mpsc::Sender<Event>, text: String) {
  if tx.try_send(Event::WsMessage(text)).is_err() {
    let _ = tx.try_send(Event::WsOverflow);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const SHUTDOWN_TEST_DEADLINE: Duration = Duration::from_secs(5);

  fn sample_summary(id: &str, is_read: bool) -> MessageSummary {
    MessageSummary {
      id: id.to_string(),
      sender: "a@test.com".to_string(),
      recipients: vec!["b@test.com".to_string()],
      subject: Some("s".to_string()),
      size: 10,
      has_attachments: false,
      is_read,
      is_starred: false,
      tags: vec![],
      created_at: "2026-04-21T00:00:00Z".to_string(),
    }
  }

  fn sample_message(id: &str) -> Message {
    Message {
      id: id.to_string(),
      sender: "a@test.com".to_string(),
      recipients: vec!["b@test.com".to_string()],
      subject: Some("s".to_string()),
      text_body: Some("body".to_string()),
      html_body: None,
      size: 10,
      has_attachments: false,
      is_read: false,
      is_starred: false,
      tags: vec![],
      created_at: "2026-04-21T00:00:00Z".to_string(),
    }
  }

  async fn spawn_recording_server() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = requests.clone();
    tokio::spawn(async move {
      loop {
        let Ok((mut socket, _)) = listener.accept().await else {
          return;
        };
        let mut buf = vec![0u8; 8192];
        let n = socket.read(&mut buf).await.unwrap_or(0);
        let head = String::from_utf8_lossy(&buf[..n]);
        if let Some(line) = head.lines().next() {
          recorded.lock().unwrap().push(line.to_string());
        }
        let _ = socket
          .write_all(
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
          )
          .await;
      }
    });
    (base_url, requests)
  }

  fn app_with_messages(count: usize) -> App {
    let (_events, event_tx) = event::channel();
    let mut app = App::new(
      "http://127.0.0.1:1".into(),
      "ws://127.0.0.1:1".into(),
      event_tx,
    );
    app.messages = (0..count)
      .map(|i| sample_summary(&format!("id-{i}"), i % 2 == 0))
      .collect();
    app.total = count as i64;
    app.sync_list_state();
    app
  }

  async fn recv_and_dispatch(app: &mut App, events: &mut event::EventHandler) {
    let event = events.next().await.expect("event channel closed");
    app.dispatch(event).await;
  }

  #[test]
  fn select_next_on_empty_list_is_noop() {
    let mut app = app_with_messages(0);
    assert!(!app.select_next());
    assert_eq!(app.selected, 0);
  }

  #[test]
  fn select_next_clamps_at_last_index() {
    let mut app = app_with_messages(3);
    app.selected = 2;
    assert!(!app.select_next(), "already at last, should return false");
    assert_eq!(app.selected, 2);
  }

  #[test]
  fn select_next_advances_from_middle() {
    let mut app = app_with_messages(5);
    app.selected = 1;
    assert!(app.select_next());
    assert_eq!(app.selected, 2);
  }

  #[test]
  fn select_prev_at_zero_stays_at_zero() {
    let mut app = app_with_messages(3);
    app.selected = 0;
    assert!(!app.select_prev());
    assert_eq!(app.selected, 0);
  }

  #[test]
  fn select_prev_on_empty_list_is_noop() {
    let mut app = app_with_messages(0);
    assert!(!app.select_prev());
    assert_eq!(app.selected, 0);
  }

  #[test]
  fn select_message_rejects_same_index() {
    let mut app = app_with_messages(3);
    app.selected = 1;
    assert!(!app.select_message(1));
  }

  #[test]
  fn select_message_rejects_out_of_bounds() {
    let mut app = app_with_messages(2);
    assert!(!app.select_message(99));
    assert_eq!(app.selected, 0);
  }

  #[test]
  fn switch_preview_tab_changes_tab() {
    let mut app = app_with_messages(1);
    assert_eq!(app.preview_tab, PreviewTab::Text);
    app.switch_preview_tab(PreviewTab::Headers);
    assert_eq!(app.preview_tab, PreviewTab::Headers);
    app.switch_preview_tab(PreviewTab::Raw);
    assert_eq!(app.preview_tab, PreviewTab::Raw);
  }

  #[test]
  fn unread_count_matches_messages() {
    // app_with_messages marks even indices as is_read=true, odd as unread.
    let app = app_with_messages(6);
    assert_eq!(app.unread_count(), 3);
  }

  #[test]
  fn current_page_reflects_offset_and_page_size() {
    let mut app = app_with_messages(0);
    app.page_size = 50;
    app.offset = 0;
    assert_eq!(app.current_page(), 1);
    app.offset = 50;
    assert_eq!(app.current_page(), 2);
    app.offset = 100;
    assert_eq!(app.current_page(), 3);
  }

  #[test]
  fn total_pages_rounds_up() {
    let mut app = app_with_messages(0);
    app.page_size = 50;
    app.total = 0;
    assert_eq!(app.total_pages(), 0);
    app.total = 1;
    assert_eq!(app.total_pages(), 1);
    app.total = 50;
    assert_eq!(app.total_pages(), 1);
    app.total = 51;
    assert_eq!(app.total_pages(), 2);
    app.total = 125;
    assert_eq!(app.total_pages(), 3);
  }

  #[tokio::test]
  async fn ws_reconnect_refetches_current_view() {
    let (base_url, requests) = spawn_recording_server().await;
    let (mut events, event_tx) = event::channel();
    let mut app = App::new(base_url, "ws://127.0.0.1:1/ws".into(), event_tx);
    app.offset = 50;
    app.search_query = "invoice".into();
    app.selected = 3;

    app.handle_ws_status(true).await;
    assert!(
      requests.lock().unwrap().is_empty(),
      "first connection must not refetch"
    );

    app.handle_ws_status(false).await;
    app.handle_ws_status(true).await;
    recv_and_dispatch(&mut app, &mut events).await;

    let requests = requests.lock().unwrap().clone();
    assert_eq!(
      requests,
      vec!["GET /api/v1/messages?limit=50&offset=50&q=invoice HTTP/1.1".to_string()]
    );
    assert_eq!(app.offset, 50);
    assert_eq!(app.search_query, "invoice");
    assert_eq!(app.selected, 3);
    assert!(app.ws_connected);
  }

  fn ws_event(kind: &str, data: serde_json::Value) -> String {
    serde_json::json!({ "type": kind, "data": data }).to_string()
  }

  fn new_message_event(id: &str) -> String {
    ws_event(
      "message:new",
      serde_json::to_value(sample_summary(id, false)).unwrap(),
    )
  }

  #[tokio::test]
  async fn live_message_is_not_merged_into_search_results() {
    let mut app = app_with_messages(3);
    app.search_query = "invoice".into();

    app.handle_ws_message(&new_message_event("live")).await;

    assert_eq!(app.total, 3);
    assert_eq!(app.messages.len(), 3);
    assert!(app.messages.iter().all(|m| m.id != "live"));
    assert!(app.view_stale);
  }

  #[tokio::test]
  async fn live_message_is_inserted_without_search() {
    let mut app = app_with_messages(3);

    app.handle_ws_message(&new_message_event("live")).await;

    assert_eq!(app.total, 4);
    assert_eq!(app.messages[0].id, "live");
    assert!(!app.view_stale);
  }

  #[tokio::test]
  async fn delete_outside_search_results_keeps_total() {
    let mut app = app_with_messages(3);
    app.search_query = "invoice".into();

    app
      .handle_ws_message(&ws_event(
        "message:delete",
        serde_json::json!({ "id": "elsewhere" }),
      ))
      .await;

    assert_eq!(app.total, 3);
  }

  #[tokio::test]
  async fn delete_inside_search_results_decrements_total() {
    let mut app = app_with_messages(3);
    app.search_query = "invoice".into();
    app.selected = 2;

    app
      .handle_ws_message(&ws_event(
        "message:delete",
        serde_json::json!({ "id": "id-0" }),
      ))
      .await;

    assert_eq!(app.total, 2);
    assert_eq!(app.messages.len(), 2);
  }

  #[tokio::test]
  async fn delete_without_search_decrements_total_even_if_not_listed() {
    let mut app = app_with_messages(3);
    app.offset = 50;

    app
      .handle_ws_message(&ws_event(
        "message:delete",
        serde_json::json!({ "id": "elsewhere" }),
      ))
      .await;

    assert_eq!(app.total, 2);
  }

  #[tokio::test]
  async fn stale_search_view_refetches_only_after_throttle() {
    let (base_url, requests) = spawn_recording_server().await;
    let (mut events, event_tx) = event::channel();
    let mut app = App::new(base_url, "ws://127.0.0.1:1/ws".into(), event_tx);
    app.search_query = "invoice".into();
    app.view_stale = true;
    app.last_fetch_at = Some(Instant::now());

    app.refetch_if_stale().await;
    assert!(
      requests.lock().unwrap().is_empty(),
      "throttle window not elapsed"
    );

    app.last_fetch_at = Instant::now().checked_sub(STALE_VIEW_REFETCH_INTERVAL);
    app.refetch_if_stale().await;
    recv_and_dispatch(&mut app, &mut events).await;

    assert_eq!(
      requests.lock().unwrap().clone(),
      vec!["GET /api/v1/messages?limit=50&offset=0&q=invoice HTTP/1.1".to_string()]
    );
  }

  #[tokio::test]
  async fn fresh_view_does_not_refetch_on_tick() {
    let (base_url, requests) = spawn_recording_server().await;
    let (_events, event_tx) = event::channel();
    let mut app = App::new(base_url, "ws://127.0.0.1:1/ws".into(), event_tx);

    app.refetch_if_stale().await;

    assert!(requests.lock().unwrap().is_empty());
  }

  #[tokio::test]
  async fn show_raw_requests_a_capped_preview() {
    let (base_url, requests) = spawn_recording_server().await;
    let (mut events, event_tx) = event::channel();
    let mut app = App::new(base_url, "ws://127.0.0.1:1/ws".into(), event_tx);
    app.messages = vec![sample_summary("id-0", true)];

    app.show_raw().await;
    recv_and_dispatch(&mut app, &mut events).await;

    assert_eq!(
      requests.lock().unwrap().clone(),
      vec![format!(
        "GET /api/v1/messages/id-0/raw?limit={RAW_PREVIEW_LIMIT_BYTES} HTTP/1.1"
      )]
    );
  }

  #[tokio::test]
  async fn messages_clear_resets_raw_preview_and_notice() {
    let mut app = app_with_messages(3);
    app.preview_raw = Some("raw body".into());
    app.preview_raw_notice = Some("truncated".into());

    app
      .handle_ws_message(&ws_event("messages:clear", serde_json::Value::Null))
      .await;

    assert_eq!(app.preview_raw, None);
    assert_eq!(app.preview_raw_notice, None);
  }

  #[test]
  fn raw_notice_is_absent_when_message_fits_the_preview() {
    assert_eq!(
      raw_truncation_notice(RAW_PREVIEW_LIMIT_BYTES, "http://x/export"),
      None
    );
  }

  #[test]
  fn raw_notice_names_both_sizes_and_the_export_url() {
    let notice = raw_truncation_notice(25 * 1024 * 1024, "http://x/api/v1/messages/id-0/export")
      .expect("oversized message must carry a notice");
    assert_eq!(
      notice,
      "Showing the first 128.0K of 25.0M. Full source: http://x/api/v1/messages/id-0/export"
    );
  }

  #[tokio::test]
  async fn ws_overflow_marks_the_view_stale() {
    let mut app = app_with_messages(1);
    assert!(!app.view_stale);

    app.dispatch(Event::WsOverflow).await;

    assert!(app.view_stale);
  }

  #[tokio::test]
  async fn ws_frame_forwarding_delivers_when_the_queue_has_room() {
    let (tx, mut rx) = mpsc::channel::<Event>(1);
    forward_ws_frame(&tx, "first".into());

    match rx.recv().await.unwrap() {
      Event::WsMessage(text) => assert_eq!(text, "first"),
      other => panic!("unexpected event: {other:?}"),
    }
  }

  #[tokio::test]
  async fn ws_frame_forwarding_drops_the_frame_when_the_queue_stays_full() {
    let (tx, rx) = mpsc::channel::<Event>(1);
    tx.try_send(Event::Tick).unwrap();

    forward_ws_frame(&tx, "dropped".into());

    let mut rx = rx;
    let mut remaining = Vec::new();
    while let Ok(event) = rx.try_recv() {
      remaining.push(event);
    }
    assert_eq!(remaining.len(), 1);
    assert!(matches!(remaining[0], Event::Tick));
  }

  #[tokio::test]
  async fn stale_fetch_result_is_discarded_after_a_newer_fetch_starts() {
    let mut app = app_with_messages(1);
    app.fetch_generation = 5;
    app.loading = true;

    let stale = ListResponse {
      messages: vec![sample_summary("stale", false)],
      total: 999,
    };
    app
      .dispatch(Event::MessagesFetched {
        generation: 4,
        result: Ok(stale),
      })
      .await;

    assert_eq!(app.total, 1);
    assert_eq!(app.messages.len(), 1);
    assert!(app.loading, "the newer, still in-flight fetch owns loading");
  }

  #[tokio::test]
  async fn matching_generation_fetch_result_updates_state() {
    let mut app = app_with_messages(0);
    app.fetch_generation = 1;
    app.loading = true;

    let resp = ListResponse {
      messages: vec![sample_summary("id-9", true)],
      total: 1,
    };
    app
      .dispatch(Event::MessagesFetched {
        generation: 1,
        result: Ok(resp),
      })
      .await;

    assert_eq!(app.total, 1);
    assert_eq!(app.messages[0].id, "id-9");
    assert!(!app.loading);
  }

  fn in_flight_fetch(count: usize) -> App {
    let mut app = app_with_messages(count);
    app.fetch_generation = 1;
    app.loading = true;
    app
  }

  async fn land_snapshot(app: &mut App, ids: &[&str], total: i64) {
    let resp = ListResponse {
      messages: ids.iter().map(|id| sample_summary(id, true)).collect(),
      total,
    };
    app
      .dispatch(Event::MessagesFetched {
        generation: app.fetch_generation,
        result: Ok(resp),
      })
      .await;
  }

  fn delete_event(id: &str) -> String {
    ws_event("message:delete", serde_json::json!({ "id": id }))
  }

  #[tokio::test]
  async fn new_message_during_in_flight_fetch_survives_the_snapshot() {
    let mut app = in_flight_fetch(3);

    app.handle_ws_message(&new_message_event("live")).await;
    land_snapshot(&mut app, &["id-0", "id-1", "id-2"], 3).await;

    let ids: Vec<&str> = app.messages.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["live", "id-0", "id-1", "id-2"]);
    assert_eq!(app.total, 4);
  }

  #[tokio::test]
  async fn delete_during_in_flight_fetch_survives_the_snapshot() {
    let mut app = in_flight_fetch(3);

    app.handle_ws_message(&delete_event("id-1")).await;
    land_snapshot(&mut app, &["id-0", "id-1", "id-2"], 3).await;

    let ids: Vec<&str> = app.messages.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["id-0", "id-2"]);
    assert_eq!(app.total, 2);
  }

  #[tokio::test]
  async fn delta_already_in_the_snapshot_is_not_duplicated() {
    let mut app = in_flight_fetch(3);

    app.handle_ws_message(&new_message_event("live")).await;
    land_snapshot(&mut app, &["live", "id-0", "id-1", "id-2"], 4).await;

    assert_eq!(app.messages.iter().filter(|m| m.id == "live").count(), 1);
    assert_eq!(app.messages.len(), 4);
    assert_eq!(app.total, 4);
  }

  #[tokio::test]
  async fn update_during_in_flight_fetch_is_replayed_onto_the_snapshot() {
    let mut app = in_flight_fetch(1);

    app
      .handle_ws_message(&ws_event(
        "message:starred",
        serde_json::json!({ "id": "id-0", "is_starred": true }),
      ))
      .await;
    land_snapshot(&mut app, &["id-0"], 1).await;

    assert!(app.messages[0].is_starred);
  }

  #[tokio::test]
  async fn delta_buffer_overflow_leaves_the_view_stale_after_the_snapshot() {
    let mut app = in_flight_fetch(0);

    for i in 0..=PENDING_DELTA_CAPACITY {
      app
        .handle_ws_message(&new_message_event(&format!("live-{i}")))
        .await;
    }
    land_snapshot(&mut app, &[], 0).await;

    assert!(app.view_stale);
  }

  #[tokio::test]
  async fn ws_overflow_during_in_flight_fetch_leaves_the_view_stale() {
    let mut app = in_flight_fetch(1);

    app.dispatch(Event::WsOverflow).await;
    land_snapshot(&mut app, &["id-0"], 1).await;

    assert!(app.view_stale);
  }

  #[tokio::test]
  async fn repeat_visit_accepts_only_the_latest_preview_request() {
    let mut app = app_with_messages(2);
    app.messages[0].is_read = false;

    app.load_preview().await;
    let first_visit = app.pending_preview.expect("first visit to id-0");
    app.select_message(1);
    app.load_preview().await;
    app.messages[0].is_read = true;
    app.select_message(0);
    app.load_preview().await;
    let latest_visit = app.pending_preview.expect("second visit to id-0");

    app
      .dispatch(Event::PreviewLoaded {
        request: first_visit,
        id: "id-0".into(),
        was_unread: true,
        result: Ok(sample_message("id-0")),
      })
      .await;
    assert!(app.preview.is_none(), "superseded request must be dropped");
    assert_eq!(app.pending_preview, Some(latest_visit));

    app
      .dispatch(Event::PreviewLoaded {
        request: latest_visit,
        id: "id-0".into(),
        was_unread: false,
        result: Ok(sample_message("id-0")),
      })
      .await;
    assert!(app.preview.is_some());
    assert_eq!(app.pending_preview, None);
  }

  #[tokio::test]
  async fn repeat_visit_accepts_only_the_latest_raw_request() {
    let mut app = app_with_messages(2);

    app.show_raw().await;
    let first_visit = app.pending_raw.expect("first raw request for id-0").1;
    app.select_message(1);
    app.show_raw().await;
    app.select_message(0);
    app.show_raw().await;
    let latest_visit = app.pending_raw.expect("second raw request for id-0").1;

    app
      .dispatch(Event::RawLoaded {
        target: RawTarget::FullView,
        request: first_visit,
        id: "id-0".into(),
        size: 10,
        result: Ok("stale raw".into()),
      })
      .await;
    assert_eq!(app.raw_content, None);

    app
      .dispatch(Event::RawLoaded {
        target: RawTarget::FullView,
        request: latest_visit,
        id: "id-0".into(),
        size: 10,
        result: Ok("fresh raw".into()),
      })
      .await;
    assert_eq!(app.raw_content.as_deref(), Some("fresh raw"));
  }

  #[tokio::test]
  async fn shutdown_aborts_in_flight_http_tasks() {
    let mut app = app_with_messages(0);
    let (guard, released) = tokio::sync::oneshot::channel::<()>();

    app.spawn_and_send(async move {
      let _guard = guard;
      std::future::pending::<Event>().await
    });
    app.shutdown_tasks().await;

    assert!(released.await.is_err(), "task must be dropped by shutdown");
  }

  #[tokio::test]
  async fn shutdown_aborts_the_websocket_reconnect_loop() {
    let (mut events, event_tx) = event::channel();
    let mut app = App::new(
      "http://127.0.0.1:1".into(),
      "ws://127.0.0.1:1/ws".into(),
      event_tx,
    );

    app.connect_websocket();
    app.shutdown_tasks().await;
    drop(app);

    while events.try_next().is_some() {}
    let closed = tokio::time::timeout(SHUTDOWN_TEST_DEADLINE, events.next())
      .await
      .expect("reconnect loop still holds the event sender");
    assert!(closed.is_err());
  }

  #[tokio::test]
  async fn stale_preview_result_is_discarded_after_selection_changes() {
    let mut app = app_with_messages(2);
    app.pending_preview = Some(2);

    app
      .dispatch(Event::PreviewLoaded {
        request: 1,
        id: "id-0".into(),
        was_unread: false,
        result: Ok(sample_message("id-0")),
      })
      .await;

    assert!(app.preview.is_none());
    assert_eq!(app.last_preview_id, None);
    assert_eq!(app.pending_preview, Some(2));
  }

  #[tokio::test]
  async fn matching_preview_result_updates_state_and_marks_read() {
    let (mut events, event_tx) = event::channel();
    let mut app = App::new(
      "http://127.0.0.1:1".into(),
      "ws://127.0.0.1:1".into(),
      event_tx,
    );
    app.messages = vec![sample_summary("id-0", false)];
    app.pending_preview = Some(1);
    app.preview_loading = true;

    app
      .dispatch(Event::PreviewLoaded {
        request: 1,
        id: "id-0".into(),
        was_unread: true,
        result: Ok(sample_message("id-0")),
      })
      .await;

    assert!(!app.preview_loading);
    assert_eq!(app.pending_preview, None);
    assert_eq!(app.last_preview_id.as_deref(), Some("id-0"));
    assert!(app.preview.is_some());

    let patch_event = events.next().await.expect("patch task must report back");
    match patch_event {
      Event::Patched {
        id,
        is_read,
        is_starred,
        ..
      } => {
        assert_eq!(id, "id-0");
        assert_eq!(is_read, Some(true));
        assert_eq!(is_starred, None);
      }
      other => panic!("unexpected event: {other:?}"),
    }
  }

  #[test]
  fn patched_event_updates_the_matching_message() {
    let mut app = app_with_messages(2);

    app.handle_patched("id-0".into(), Some(true), Some(true), Ok(()));

    let msg = app.messages.iter().find(|m| m.id == "id-0").unwrap();
    assert!(msg.is_read);
    assert!(msg.is_starred);
  }

  #[test]
  fn patched_event_is_ignored_on_failure() {
    let mut app = app_with_messages(2);
    let before = app.messages[0].is_starred;

    app.handle_patched("id-0".into(), None, Some(true), Err("boom".into()));

    assert_eq!(app.messages[0].is_starred, before);
  }

  #[tokio::test]
  async fn deleted_event_removes_the_message_and_clamps_selection() {
    let mut app = app_with_messages(2);
    app.selected = 1;

    app.handle_deleted("id-1".into(), Ok(())).await;

    assert_eq!(app.messages.len(), 1);
    assert_eq!(app.selected, 0);
    assert!(app.messages.iter().all(|m| m.id != "id-1"));
  }

  #[tokio::test]
  async fn all_deleted_event_clears_the_list() {
    let mut app = app_with_messages(3);

    app.handle_all_deleted(Ok(())).await;

    assert!(app.messages.is_empty());
    assert_eq!(app.selected, 0);
  }

  #[tokio::test]
  async fn raw_loaded_event_populates_the_preview_tab_when_current() {
    let mut app = app_with_messages(1);
    app.pending_raw = Some((RawTarget::Preview, 1));

    app
      .dispatch(Event::RawLoaded {
        target: RawTarget::Preview,
        request: 1,
        id: "id-0".into(),
        size: 10,
        result: Ok("raw body".into()),
      })
      .await;

    assert_eq!(app.preview_raw.as_deref(), Some("raw body"));
    assert_eq!(app.pending_raw, None);
    assert_eq!(app.mode, Mode::Normal);
  }

  #[tokio::test]
  async fn raw_loaded_event_populates_the_full_view_when_current() {
    let mut app = app_with_messages(1);
    app.pending_raw = Some((RawTarget::FullView, 1));

    app
      .dispatch(Event::RawLoaded {
        target: RawTarget::FullView,
        request: 1,
        id: "id-0".into(),
        size: 10,
        result: Ok("raw body".into()),
      })
      .await;

    assert_eq!(app.raw_content.as_deref(), Some("raw body"));
    assert_eq!(app.mode, Mode::RawView);
  }

  #[tokio::test]
  async fn raw_loaded_event_is_discarded_when_superseded() {
    let mut app = app_with_messages(1);
    app.pending_raw = Some((RawTarget::Preview, 2));

    app
      .dispatch(Event::RawLoaded {
        target: RawTarget::Preview,
        request: 1,
        id: "id-0".into(),
        size: 10,
        result: Ok("stale raw".into()),
      })
      .await;

    assert_eq!(app.preview_raw, None);
    assert_eq!(app.pending_raw, Some((RawTarget::Preview, 2)));
  }

  fn selected_id(app: &App) -> Option<&str> {
    app.messages.get(app.selected).map(|m| m.id.as_str())
  }

  fn app_with_live_channel(count: usize) -> (App, event::EventHandler) {
    let (events, event_tx) = event::channel();
    let mut app = App::new(
      "http://127.0.0.1:1".into(),
      "ws://127.0.0.1:1".into(),
      event_tx,
    );
    app.messages = (0..count)
      .map(|i| sample_summary(&format!("id-{i}"), true))
      .collect();
    app.total = count as i64;
    app.sync_list_state();
    (app, events)
  }

  fn show_preview_of(app: &mut App, id: &str) {
    app.preview = Some(sample_message(id));
    app.last_preview_id = Some(id.to_string());
  }

  #[test]
  fn resolution_keeps_the_anchor_when_it_survives_at_a_new_position() {
    let old = ["a", "b", "c"].map(String::from);
    let new = ["x", "c", "b", "a"].map(|id| sample_summary(id, true));
    assert_eq!(resolve_selection(&old, 1, &new), 2);
  }

  #[test]
  fn resolution_picks_the_first_surviving_successor_when_the_anchor_is_gone() {
    let old = ["a", "b", "c", "d"].map(String::from);
    let new = ["a", "d"].map(|id| sample_summary(id, true));
    assert_eq!(resolve_selection(&old, 1, &new), 1);
  }

  #[test]
  fn resolution_falls_back_to_the_nearest_surviving_predecessor() {
    let old = ["a", "b", "c"].map(String::from);
    let new = ["a", "x"].map(|id| sample_summary(id, true));
    assert_eq!(resolve_selection(&old, 2, &new), 0);
  }

  #[test]
  fn resolution_clamps_when_nothing_from_the_old_list_survives() {
    let old = ["a", "b", "c"].map(String::from);
    let new = ["x", "y"].map(|id| sample_summary(id, true));
    assert_eq!(resolve_selection(&old, 2, &new), 1);
    assert_eq!(resolve_selection(&old, 2, &[]), 0);
  }

  #[tokio::test]
  async fn live_arrival_keeps_the_selected_message() {
    let mut app = app_with_messages(3);
    app.select_message(1);
    show_preview_of(&mut app, "id-1");

    app.handle_ws_message(&new_message_event("live")).await;

    assert_eq!(selected_id(&app), Some("id-1"));
    assert_eq!(app.last_preview_id.as_deref(), Some("id-1"));
    assert_eq!(app.pending_preview, None);
  }

  #[tokio::test]
  async fn live_arrival_that_pushes_the_selection_off_the_page_reloads_the_preview() {
    let mut app = app_with_messages(3);
    app.page_size = 3;
    app.select_message(2);
    show_preview_of(&mut app, "id-2");

    app.handle_ws_message(&new_message_event("live")).await;

    assert_eq!(selected_id(&app), Some("id-1"));
    assert!(app.preview.is_none(), "preview of an off-page mail must go");
    assert!(app.pending_preview.is_some());
  }

  #[tokio::test]
  async fn live_delete_above_the_selection_keeps_the_selected_message() {
    let mut app = app_with_messages(3);
    app.select_message(1);
    show_preview_of(&mut app, "id-1");

    app.handle_ws_message(&delete_event("id-0")).await;

    assert_eq!(selected_id(&app), Some("id-1"));
    assert_eq!(app.last_preview_id.as_deref(), Some("id-1"));
    assert_eq!(app.pending_preview, None);
  }

  #[tokio::test]
  async fn live_delete_of_the_selection_moves_to_the_row_that_took_its_place() {
    let mut app = app_with_messages(3);
    app.select_message(1);
    show_preview_of(&mut app, "id-1");

    app.handle_ws_message(&delete_event("id-1")).await;

    assert_eq!(selected_id(&app), Some("id-2"));
    assert!(
      app.preview.is_none(),
      "deleted mail must not stay on screen"
    );
    assert!(app.pending_preview.is_some());
  }

  #[tokio::test]
  async fn live_delete_of_the_last_selected_row_clamps_to_the_new_last() {
    let mut app = app_with_messages(3);
    app.select_message(2);

    app.handle_ws_message(&delete_event("id-2")).await;

    assert_eq!(selected_id(&app), Some("id-1"));
  }

  #[tokio::test]
  async fn confirmed_delete_of_another_message_keeps_the_selection_and_preview() {
    let mut app = app_with_messages(3);
    app.select_message(1);
    show_preview_of(&mut app, "id-1");

    app.handle_deleted("id-0".into(), Ok(())).await;

    assert_eq!(selected_id(&app), Some("id-1"));
    assert_eq!(app.last_preview_id.as_deref(), Some("id-1"));
    assert!(app.preview.is_some());
  }

  #[tokio::test]
  async fn resync_with_a_reordered_list_keeps_the_selected_message() {
    let mut app = in_flight_fetch(3);
    app.select_message(0);
    show_preview_of(&mut app, "id-0");

    land_snapshot(&mut app, &["id-2", "id-1", "id-0"], 3).await;

    assert_eq!(selected_id(&app), Some("id-0"));
    assert_eq!(app.pending_preview, None);
  }

  #[tokio::test]
  async fn resync_that_drops_the_selection_picks_its_surviving_successor() {
    let mut app = in_flight_fetch(4);
    app.select_message(1);

    land_snapshot(&mut app, &["id-2", "id-3"], 2).await;

    assert_eq!(selected_id(&app), Some("id-2"));
  }

  #[tokio::test]
  async fn arrival_during_in_flight_fetch_keeps_the_selection_after_replay() {
    let mut app = in_flight_fetch(3);
    app.select_message(0);

    app.handle_ws_message(&new_message_event("live")).await;
    land_snapshot(&mut app, &["id-1", "id-0", "id-2"], 3).await;

    assert_eq!(selected_id(&app), Some("id-0"));
  }

  #[tokio::test]
  async fn filter_change_keeps_the_selection_when_it_still_matches() {
    let mut app = app_with_messages(3);
    app.select_message(1);
    app.search_input = "invoice".into();

    app.handle_search_key(KeyEvent::from(KeyCode::Enter)).await;
    assert_eq!(
      selected_id(&app),
      Some("id-1"),
      "cursor holds until results land"
    );
    land_snapshot(&mut app, &["id-7", "id-8", "id-1"], 3).await;

    assert_eq!(selected_id(&app), Some("id-1"));
  }

  #[tokio::test]
  async fn filter_change_selects_the_top_row_when_the_selection_no_longer_matches() {
    let mut app = app_with_messages(3);
    app.select_message(2);
    app.search_input = "invoice".into();

    app.handle_search_key(KeyEvent::from(KeyCode::Enter)).await;
    land_snapshot(&mut app, &["id-7", "id-8", "id-9"], 3).await;

    assert_eq!(selected_id(&app), Some("id-7"));
  }

  #[tokio::test]
  async fn page_change_selects_the_top_row_of_the_new_page() {
    let mut app = app_with_messages(3);
    app.total = 120;
    app.select_message(2);

    app.next_page().await;
    land_snapshot(&mut app, &["id-50", "id-51"], 120).await;

    assert_eq!(selected_id(&app), Some("id-50"));
  }

  #[tokio::test]
  async fn deltas_during_an_in_flight_fetch_load_one_preview_once_it_lands() {
    let mut app = in_flight_fetch(4);
    app.select_message(1);
    show_preview_of(&mut app, "id-1");
    let before = app.last_request_id;

    app.handle_ws_message(&delete_event("id-1")).await;
    app.handle_ws_message(&new_message_event("live")).await;
    app.handle_ws_message(&delete_event("id-2")).await;
    assert_eq!(app.last_request_id, before, "no preview load mid-fetch");
    assert_eq!(app.pending_preview, None);

    land_snapshot(&mut app, &["id-0", "id-1", "id-2", "id-3"], 4).await;

    assert_eq!(selected_id(&app), Some("id-3"));
    assert_eq!(app.last_request_id, before + 1);
    assert_eq!(app.pending_preview, Some(before + 1));
    assert!(
      app.preview.is_none(),
      "deleted mail must not stay on screen"
    );
  }

  #[tokio::test]
  async fn failed_view_change_fetch_does_not_leak_into_the_next_resync() {
    let mut app = app_with_messages(4);
    app.select_message(1);
    app.search_input = "invoice".into();

    app.handle_search_key(KeyEvent::from(KeyCode::Enter)).await;
    app
      .dispatch(Event::MessagesFetched {
        generation: app.fetch_generation,
        result: Err("boom".into()),
      })
      .await;
    app.fetch_messages().await;
    land_snapshot(&mut app, &["id-0", "id-2", "id-3"], 3).await;

    assert_eq!(selected_id(&app), Some("id-2"));
  }

  #[tokio::test]
  async fn star_after_a_delete_above_targets_the_selected_message() {
    let (mut app, mut events) = app_with_live_channel(3);
    app.select_message(1);

    app.handle_ws_message(&delete_event("id-0")).await;
    app.toggle_star().await;

    match events.next().await.expect("patch task must report back") {
      Event::Patched { id, is_starred, .. } => {
        assert_eq!(id, "id-1");
        assert_eq!(is_starred, Some(true));
      }
      other => panic!("unexpected event: {other:?}"),
    }
  }
}
