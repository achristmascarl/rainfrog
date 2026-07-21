use color_eyre::eyre::Result;
use crossterm::event::{KeyEvent, MouseEvent, MouseEventKind};
use ratatui::prelude::*;
use ratatui_textarea::{CursorMove, Input, Key, TextArea};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::mpsc::{self, UnboundedReceiver};

use super::{Component, Frame};
use crate::{
  action::Action,
  app::AppState,
  completion::{
    CompletionCandidate, CompletionClient, CompletionCommand, CompletionKind, CompletionRequest,
    CompletionResponse, CompletionUiEvent, CursorPosition, current_replacement_range,
    render::{ViewportState, cursor_anchor, render_dropdown},
  },
  config::Config,
  database::get_keywords,
  focus::Focus,
  tui::Event,
  vim::{Mode, Transition, Vim},
};

fn keyword_regex() -> String {
  format!("(?i)(^|[^a-zA-Z0-9\'\"`._]+)({})($|[^a-zA-Z0-9\'\"`._]+)", get_keywords().join("|"))
}

#[derive(Default)]
struct CompletionMenuState {
  candidates: Vec<CompletionCandidate>,
  selected: usize,
  displayed_generation: u64,
  pending_generation: u64,
  dismissed_generation: Option<u64>,
}

impl CompletionMenuState {
  fn is_visible(&self) -> bool {
    !self.candidates.is_empty()
  }

  fn hide(&mut self) {
    self.candidates.clear();
    self.selected = 0;
  }

  fn dismiss(&mut self) {
    self.dismissed_generation = Some(self.pending_generation.max(self.displayed_generation));
    self.hide();
  }

  fn apply(&mut self, response: CompletionResponse) {
    if response.generation < self.pending_generation
      || self.dismissed_generation == Some(response.generation)
    {
      return;
    }
    let selected_identity = self
      .candidates
      .get(self.selected)
      .map(|candidate| (candidate.source, candidate.kind, candidate.insert_text.clone()));
    let old_selected = self.selected;
    self.candidates = response.candidates;
    self.displayed_generation = response.generation;
    self.pending_generation = self.pending_generation.max(response.generation);
    self.selected = selected_identity
      .and_then(|identity| {
        self.candidates.iter().position(|candidate| {
          (candidate.source, candidate.kind, candidate.insert_text.clone()) == identity
        })
      })
      .unwrap_or_else(|| old_selected.min(self.candidates.len().saturating_sub(1)));
  }

  fn next(&mut self) {
    if !self.candidates.is_empty() {
      self.selected = (self.selected + 1) % self.candidates.len();
    }
  }

  fn previous(&mut self) {
    if !self.candidates.is_empty() {
      self.selected = (self.selected + self.candidates.len() - 1) % self.candidates.len();
    }
  }
}

pub struct Editor<'a> {
  command_tx: Option<UnboundedSender<Action>>,
  config: Config,
  textarea: TextArea<'a>,
  vim_state: Vim,
  cursor_style: Style,
  last_query_duration: Option<chrono::Duration>,
  completion: CompletionMenuState,
  completion_viewport: ViewportState,
  completion_command_tx: UnboundedSender<CompletionCommand>,
  completion_response_rx: UnboundedReceiver<CompletionUiEvent>,
  suppress_next_completion_request: bool,
  pending_request_snapshot: Option<(u64, String, CursorPosition)>,
}

impl Default for Editor<'_> {
  fn default() -> Self {
    Self::new()
  }
}

impl Editor<'_> {
  pub fn new() -> Self {
    let (completion_command_tx, _) = mpsc::unbounded_channel();
    let (_, completion_response_rx) = mpsc::unbounded_channel();
    Self::with_completion_channels(CompletionClient {
      command_tx: completion_command_tx,
      response_rx: completion_response_rx,
    })
  }

  pub fn with_completion_channels(client: CompletionClient) -> Self {
    let mut textarea = TextArea::default();
    textarea.set_search_pattern(keyword_regex()).unwrap();
    Editor {
      command_tx: None,
      config: Config::default(),
      textarea,
      vim_state: Vim::new(Mode::Normal),
      cursor_style: Mode::Normal.cursor_style(),
      last_query_duration: None,
      completion: CompletionMenuState::default(),
      completion_viewport: ViewportState::default(),
      completion_command_tx: client.command_tx,
      completion_response_rx: client.response_rx,
      suppress_next_completion_request: false,
      pending_request_snapshot: None,
    }
  }

  pub fn apply_completion_response(&mut self, response: CompletionResponse) {
    if let Some((generation, text, cursor)) = &self.pending_request_snapshot {
      let live_cursor = self.textarea.cursor();
      if response.generation != *generation
        || self.textarea.lines().join("\n") != *text
        || (live_cursor.0, live_cursor.1) != (cursor.row, cursor.col)
        || self.vim_state.mode != Mode::Insert
      {
        return;
      }
    }
    self.completion.apply(response);
  }

  fn drain_completion_responses(&mut self) {
    while let Ok(event) = self.completion_response_rx.try_recv() {
      match event {
        CompletionUiEvent::Response(response) => self.apply_completion_response(response),
      }
    }
  }

  fn request_completion(&mut self, app_state: &AppState, manual: bool) {
    if !self.config.settings.autocomplete_enabled.unwrap_or(true)
      || self.vim_state.mode != Mode::Insert
      || app_state.focus != Focus::Editor
    {
      return;
    }
    self.completion.pending_generation = self.completion.pending_generation.saturating_add(1);
    if manual {
      self.completion.dismissed_generation = None;
    }
    let cursor = self.textarea.cursor();
    let text = self.textarea.lines().join("\n");
    self.pending_request_snapshot = Some((
      self.completion.pending_generation,
      text.clone(),
      CursorPosition { row: cursor.0, col: cursor.1 },
    ));
    let _ = self.completion_command_tx.send(CompletionCommand::Request(CompletionRequest {
      generation: self.completion.pending_generation,
      text,
      cursor: CursorPosition { row: cursor.0, col: cursor.1 },
      manual,
      driver: app_state.driver,
    }));
  }

  fn cancel_completion(&mut self) {
    let generation = self.completion.pending_generation.max(self.completion.displayed_generation);
    let _ = self.completion_command_tx.send(CompletionCommand::Cancel { generation });
  }

  fn handle_completion_input(&mut self, input: Input) -> bool {
    if !self.completion.is_visible() || self.vim_state.mode != Mode::Insert {
      return false;
    }
    match input {
      Input { key: Key::Esc, .. } => {
        self.completion.dismiss();
        self.cancel_completion();
        true
      },
      Input { key: Key::Up, .. } | Input { key: Key::Char('p'), ctrl: true, .. } => {
        self.completion.previous();
        true
      },
      Input { key: Key::Down, .. } | Input { key: Key::Char('n'), ctrl: true, .. } => {
        self.completion.next();
        true
      },
      Input { key: Key::Tab, .. }
      | Input { key: Key::Enter, ctrl: false, alt: false, shift: false } => {
        self.accept_completion();
        true
      },
      _ => false,
    }
  }

  fn accept_completion(&mut self) {
    let Some(candidate) = self.completion.candidates.get(self.completion.selected).cloned() else {
      return;
    };
    let completes_directory =
      candidate.kind == CompletionKind::Path && candidate.insert_text.ends_with('/');
    let text = self.textarea.lines().join("\n");
    let cursor = self.textarea.cursor();
    let range = current_replacement_range(&text, CursorPosition { row: cursor.0, col: cursor.1 });
    if range.start.row == cursor.0 && range.end.row == cursor.0 && range.start.col <= cursor.1 {
      self.textarea.start_selection();
      for _ in range.start.col..cursor.1 {
        self.textarea.move_cursor(CursorMove::Back);
      }
      self.textarea.insert_str(candidate.insert_text);
    }
    self.suppress_next_completion_request = !completes_directory;
    self.completion.hide();
  }

  pub fn transition_vim_state(&mut self, input: Input, app_state: &AppState) -> Result<()> {
    if self.handle_completion_input(input.clone()) {
      return Ok(());
    }
    let previous_mode = self.vim_state.mode;
    match input {
      Input { key: Key::Enter, alt: true, .. } | Input { key: Key::Enter, ctrl: true, .. } => {
        if !app_state.query_task_running
          && let Some(sender) = &self.command_tx
        {
          sender.send(Action::Query(self.textarea.lines().to_vec(), false, false))?;
          self.vim_state = Vim::new(Mode::Normal);
          self.vim_state.register_action_handler(self.command_tx.clone())?;
          self.cursor_style = Mode::Normal.cursor_style();
        }
      },
      Input { key: Key::Tab, shift: false, .. } if self.vim_state.mode != Mode::Insert => {
        if let Some(sender) = &self.command_tx {
          sender.send(Action::CycleFocusForwards)?;
        }
      },
      Input { key: Key::Char('f'), ctrl: true, .. } if self.vim_state.mode != Mode::Insert => {
        if let Some(sender) = &self.command_tx {
          sender.send(Action::RequestSaveFavorite(self.textarea.lines().to_vec()))?;
        }
      },
      Input { key: Key::Char('f'), alt: true, .. } => {
        if let Some(sender) = &self.command_tx {
          sender.send(Action::RequestSaveFavorite(self.textarea.lines().to_vec()))?;
        }
      },
      Input { key: Key::Char('c'), ctrl: true, .. }
        if matches!(self.vim_state.mode, Mode::Normal) =>
      {
        if let Some(sender) = &self.command_tx {
          sender.send(Action::Quit)?;
        }
      },
      Input { key: Key::Char('q'), .. } if matches!(self.vim_state.mode, Mode::Normal) => {
        if let Some(sender) = &self.command_tx {
          sender.send(Action::AbortQuery)?;
        }
      },
      _ => {
        let new_vim_state = self.vim_state.clone();
        self.vim_state = match new_vim_state.transition(input, &mut self.textarea) {
          Transition::Mode(mode) if new_vim_state.mode != mode => {
            self.cursor_style = mode.cursor_style();
            Vim::new(mode)
          },
          Transition::Nop | Transition::Mode(_) => new_vim_state,
          Transition::Pending(input) => new_vim_state.with_pending(input),
        };
        self.vim_state.register_action_handler(self.command_tx.clone())?;
      },
    };
    if previous_mode == Mode::Insert && self.vim_state.mode != Mode::Insert {
      self.completion.dismiss();
      self.cancel_completion();
    }
    Ok(())
  }
}

impl Component for Editor<'_> {
  fn register_action_handler(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
    self.vim_state.register_action_handler(self.command_tx.clone())?;
    self.command_tx = Some(tx);
    Ok(())
  }

  fn register_config_handler(&mut self, config: Config) -> Result<()> {
    self.config = config;
    Ok(())
  }

  fn handle_mouse_events(
    &mut self,
    mouse: MouseEvent,
    app_state: &AppState,
  ) -> Result<Option<Action>> {
    if app_state.focus != Focus::Editor {
      return Ok(None);
    }
    match mouse.kind {
      MouseEventKind::ScrollDown => {
        self.textarea.scroll((1, 0));
        self.completion_viewport.scroll(1, 0);
      },
      MouseEventKind::ScrollUp => {
        self.textarea.scroll((-1, 0));
        self.completion_viewport.scroll(-1, 0);
      },
      MouseEventKind::ScrollLeft => {
        self.transition_vim_state(
          Input { key: Key::Char('h'), ctrl: false, alt: false, shift: false },
          app_state,
        )?;
      },
      MouseEventKind::ScrollRight => {
        self.transition_vim_state(
          Input { key: Key::Char('j'), ctrl: false, alt: false, shift: false },
          app_state,
        )?;
      },
      _ => {},
    };
    Ok(None)
  }

  fn handle_events(
    &mut self,
    event: Option<Event>,
    last_tick_key_events: Vec<KeyEvent>,
    app_state: &AppState,
  ) -> Result<Option<Action>> {
    if app_state.focus != Focus::Editor {
      return Ok(None);
    }
    self.drain_completion_responses();
    let before = self.textarea.lines().to_vec();
    let cursor_before = self.textarea.cursor();
    if let Some(Event::Paste(text)) = event {
      self.textarea.insert_str(text);
    } else if let Some(Event::Mouse(event)) = event {
      self.handle_mouse_events(event, app_state).unwrap();
    } else if let Some(Event::Key(key)) = event {
      let input = Input::from(key);
      self.transition_vim_state(input, app_state)?;
    };
    if self.vim_state.mode == Mode::Insert && self.textarea.lines() != before {
      if self.suppress_next_completion_request {
        self.suppress_next_completion_request = false;
      } else {
        self.request_completion(app_state, false);
      }
    } else if self.textarea.cursor() != cursor_before
      && (self.completion.is_visible() || self.pending_request_snapshot.is_some())
    {
      self.completion.dismiss();
      self.cancel_completion();
      self.pending_request_snapshot = None;
    }
    Ok(None)
  }

  fn update(&mut self, action: Action, app_state: &AppState) -> Result<Option<Action>> {
    self.drain_completion_responses();
    if app_state.focus != Focus::Editor && self.completion.is_visible() {
      self.completion.dismiss();
      self.cancel_completion();
    }
    match action {
      Action::TriggerCompletion => self.request_completion(app_state, true),
      Action::SubmitEditorQueryBypassParser => {
        if let Some(sender) = &self.command_tx {
          sender.send(Action::Query(self.textarea.lines().to_vec(), false, true))?;
        }
      },
      Action::SubmitEditorQuery => {
        if let Some(sender) = &self.command_tx {
          sender.send(Action::Query(self.textarea.lines().to_vec(), false, false))?;
        }
      },
      Action::QueryToEditor(lines) => {
        self.completion.dismiss();
        self.cancel_completion();
        self.textarea = TextArea::from(lines.clone());
        self.textarea.set_search_pattern(keyword_regex()).unwrap();
      },
      Action::CopyData(data) => {
        self.textarea.set_yank_text(data);
      },
      _ => {},
    }
    Ok(None)
  }

  fn draw(&mut self, f: &mut Frame<'_>, area: Rect, app_state: &AppState) -> Result<()> {
    self.drain_completion_responses();
    let focused = app_state.focus == Focus::Editor;

    if let Some(query_start) = app_state.last_query_start {
      self.last_query_duration = match app_state.last_query_end {
        Some(end) => Some(end.signed_duration_since(query_start)),
        None => Some(chrono::Utc::now().signed_duration_since(query_start)),
      };
    }

    let duration_string = self.last_query_duration.map_or("".to_string(), |d| {
      let seconds: f64 = (d.num_milliseconds()
        % std::cmp::max(1, d.num_minutes()).saturating_mul(60).saturating_mul(1000))
        as f64
        / 1000_f64;
      format!(
        " {}{}:{}{:.3}s ",
        if d.num_minutes() < 10 { "0" } else { "" },
        d.num_minutes(),
        if seconds < 10.0 { "0" } else { "" },
        seconds
      )
    });
    let block = self
      .vim_state
      .mode
      .block()
      .border_style(if focused { Style::new().green() } else { Style::new().dim() })
      .title(Line::from(duration_string).right_aligned());
    let inner = block.inner(area);

    self.textarea.set_cursor_style(self.cursor_style);
    self.textarea.set_block(block);
    self.textarea.set_line_number_style(if focused {
      Style::default().fg(Color::Yellow)
    } else {
      Style::new().dim()
    });
    self.textarea.set_cursor_line_style(Style::default().not_underlined());
    self.textarea.set_hard_tab_indent(false);
    self.textarea.set_tab_length(2);
    self.textarea.set_search_style(Style::default().fg(Color::Magenta).bold());
    f.render_widget(&self.textarea, area);
    if focused && self.vim_state.mode == Mode::Insert && self.completion.is_visible() {
      let screen_cursor = self.textarea.screen_cursor();
      let gutter_width = self.textarea.lines().len().max(1).ilog10() as u16 + 3;
      self.completion_viewport.keep_cursor_visible(screen_cursor, inner, gutter_width);
      if let Some(anchor) =
        cursor_anchor(inner, screen_cursor, self.completion_viewport, gutter_width)
      {
        render_dropdown(f, area, anchor, &self.completion.candidates, self.completion.selected);
      }
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::completion::{CompletionKind, CompletionSource, CursorPosition, TextRange};
  use crate::{components::app_state_with_focus, tui::Event};
  use crossterm::event::KeyCode;

  fn response(candidates: &[&str]) -> CompletionResponse {
    CompletionResponse {
      generation: 1,
      replacement_range: TextRange {
        start: CursorPosition { row: 0, col: 0 },
        end: CursorPosition { row: 0, col: 0 },
      },
      candidates: candidates
        .iter()
        .map(|candidate| {
          CompletionCandidate::new(*candidate, CompletionKind::Keyword, CompletionSource::SqlSyntax)
        })
        .collect(),
      missing_columns: Vec::new(),
    }
  }

  #[test]
  fn paste_keeps_multiline_editor_input() {
    let mut editor = Editor::new();

    editor
      .handle_events(
        Some(Event::Paste("select 1;\nselect 2;".to_string())),
        Vec::new(),
        &app_state_with_focus(Focus::Editor),
      )
      .unwrap();

    assert_eq!(editor.textarea.lines(), &["select 1;", "select 2;"]);
  }

  #[test]
  fn completion_accepts_against_live_token_range() {
    let mut editor = Editor::new();
    editor.vim_state = Vim::new(Mode::Insert);
    editor.textarea.insert_str("sel");
    editor.apply_completion_response(response(&["SELECT"]));

    editor
      .transition_vim_state(
        Input { key: Key::Tab, ctrl: false, alt: false, shift: false },
        &app_state_with_focus(Focus::Editor),
      )
      .unwrap();

    assert_eq!(editor.textarea.lines(), &["SELECT"]);
    assert!(!editor.completion.is_visible());
  }

  #[test]
  fn completion_accepts_insert_text_while_displaying_label() {
    let mut editor = Editor::new();
    editor.vim_state = Vim::new(Mode::Insert);
    editor.textarea.insert_str("full");
    let mut completion_response = response(&[]);
    completion_response.candidates.push(
      CompletionCandidate::new("full name", CompletionKind::Column, CompletionSource::Database)
        .with_insert_text("\"full name\""),
    );
    editor.apply_completion_response(completion_response);

    assert_eq!(editor.completion.candidates[0].label, "full name");
    editor
      .transition_vim_state(
        Input { key: Key::Tab, ctrl: false, alt: false, shift: false },
        &app_state_with_focus(Focus::Editor),
      )
      .unwrap();

    assert_eq!(editor.textarea.lines(), &["\"full name\""]);
  }

  #[test]
  fn modified_enter_executes_query_instead_of_accepting_completion() {
    let mut editor = Editor::new();
    editor.vim_state = Vim::new(Mode::Insert);
    editor.textarea.insert_str("sel");
    editor.apply_completion_response(response(&["SELECT"]));
    let (action_tx, mut action_rx) = mpsc::unbounded_channel();
    editor.register_action_handler(action_tx).unwrap();

    editor
      .transition_vim_state(
        Input { key: Key::Enter, ctrl: false, alt: true, shift: false },
        &app_state_with_focus(Focus::Editor),
      )
      .unwrap();

    assert_eq!(editor.textarea.lines(), &["sel"]);
    assert_eq!(action_rx.try_recv().unwrap(), Action::Query(vec!["sel".into()], false, false));
    assert_eq!(editor.vim_state.mode, Mode::Normal);
    assert!(!editor.completion.is_visible());
  }

  #[test]
  fn completion_escape_dismisses_without_leaving_insert_mode() {
    let mut editor = Editor::new();
    editor.vim_state = Vim::new(Mode::Insert);
    editor.apply_completion_response(response(&["SELECT"]));

    editor
      .transition_vim_state(
        Input { key: Key::Esc, ctrl: false, alt: false, shift: false },
        &app_state_with_focus(Focus::Editor),
      )
      .unwrap();

    assert_eq!(editor.vim_state.mode, Mode::Insert);
    assert!(!editor.completion.is_visible());
  }

  #[test]
  fn editing_keeps_existing_completion_visible() {
    let mut editor = Editor::new();
    editor.vim_state = Vim::new(Mode::Insert);
    editor.apply_completion_response(response(&["SELECT"]));

    editor
      .handle_events(
        Some(Event::Key(KeyEvent::new(KeyCode::Char('x'), crossterm::event::KeyModifiers::NONE))),
        Vec::new(),
        &app_state_with_focus(Focus::Editor),
      )
      .unwrap();

    assert!(editor.completion.is_visible());
    assert_eq!(editor.textarea.lines(), &["x"]);
  }

  #[test]
  fn cursor_movement_dismisses_existing_completion() {
    let mut editor = Editor::new();
    editor.vim_state = Vim::new(Mode::Insert);
    editor.textarea.insert_str("sel");
    editor.apply_completion_response(response(&["SELECT"]));

    editor
      .handle_events(
        Some(Event::Key(KeyEvent::new(KeyCode::Left, crossterm::event::KeyModifiers::NONE))),
        Vec::new(),
        &app_state_with_focus(Focus::Editor),
      )
      .unwrap();

    assert_eq!(editor.textarea.cursor(), (0, 2));
    assert_eq!(editor.textarea.lines(), &["sel"]);
    assert!(!editor.completion.is_visible());
  }
}
