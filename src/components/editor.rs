use std::{
  collections::HashMap,
  sync::{Arc, Mutex},
  time::Duration,
};

#[cfg(not(feature = "termux"))]
use arboard::Clipboard;
use color_eyre::eyre::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseEvent, MouseEventKind};
use ratatui::{prelude::*, widgets::*};
use serde::{Deserialize, Serialize};
use sqlx::{Database, Executor, Pool};
use tokio::sync::mpsc::UnboundedSender;
use tui_textarea::{Input, Key, Scrolling, TextArea};

use super::{Component, Frame};
use crate::{
  action::{Action, MenuPreview},
  app::{App, AppState, DbTask},
  config::{Config, KeyBindings},
  database::{self, get_keywords, DatabaseQueries, HasRowsAffected, ValueParser},
  focus::Focus,
  tui::Event,
  vim::{Mode, Transition, Vim},
};

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
struct CursorPosition {
  pub row: u32,
  pub col: u32,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
struct Selection {
  pub start: CursorPosition,
  pub end: CursorPosition,
}

fn keyword_regex() -> String {
  format!("(?i)(^|[^a-zA-Z0-9\'\"`._]+)({})($|[^a-zA-Z0-9\'\"`._]+)", get_keywords().join("|"))
}

#[derive(Default)]
pub struct Editor<'a> {
  command_tx: Option<UnboundedSender<Action>>,
  config: Config,
  selection: Option<Selection>,
  textarea: TextArea<'a>,
  vim_state: Vim,
  cursor_style: Style,
  last_query_duration: Option<chrono::Duration>,
}

impl<'a> Editor<'a> {
  pub fn new() -> Self {
    let mut textarea = TextArea::default();
    textarea.set_search_pattern(keyword_regex()).unwrap();
    Editor {
      command_tx: None,
      config: Config::default(),
      selection: None,
      textarea,
      vim_state: Vim::new(Mode::Normal),
      cursor_style: Mode::Normal.cursor_style(),
      last_query_duration: None,
    }
  }

  pub fn transition_vim_state<DB: Database + DatabaseQueries>(
    &mut self,
    input: Input,
    app_state: &AppState<'_, DB>,
  ) -> Result<()> {
    match input {
      Input { key: Key::Enter, alt: true, .. } | Input { key: Key::Enter, ctrl: true, .. } => {
        if app_state.query_task.is_none() {
          if let Some(sender) = &self.command_tx {
            sender.send(Action::Query(self.textarea.lines().to_vec(), false))?;
            self.vim_state = Vim::new(Mode::Normal);
            self.vim_state.register_action_handler(self.command_tx.clone())?;
            self.cursor_style = Mode::Normal.cursor_style();
          }
        }
      },
      Input { key: Key::Tab, shift: false, .. } if self.vim_state.mode != Mode::Insert => {
        if let Some(sender) = &self.command_tx {
          sender.send(Action::CycleFocusForwards)?;
        }
      },
      Input { key: Key::Char('c'), ctrl: true, .. } if matches!(self.vim_state.mode, Mode::Normal) => {
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
    Ok(())
  }
}

impl<'a, DB: Database + DatabaseQueries> Component<DB> for Editor<'a> {
  fn register_action_handler(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
    self.vim_state.register_action_handler(self.command_tx.clone())?;
    self.command_tx = Some(tx);
    Ok(())
  }

  fn register_config_handler(&mut self, config: Config) -> Result<()> {
    self.config = config;
    Ok(())
  }

  fn handle_mouse_events(&mut self, mouse: MouseEvent, app_state: &AppState<'_, DB>) -> Result<Option<Action>> {
    if app_state.focus != Focus::Editor {
      return Ok(None);
    }
    match mouse.kind {
      MouseEventKind::ScrollDown => {
        self.textarea.scroll((1, 0));
      },
      MouseEventKind::ScrollUp => {
        self.textarea.scroll((-1, 0));
      },
      MouseEventKind::ScrollLeft => {
        self.transition_vim_state(Input { key: Key::Char('h'), ctrl: false, alt: false, shift: false }, app_state)?;
      },
      MouseEventKind::ScrollRight => {
        self.transition_vim_state(Input { key: Key::Char('j'), ctrl: false, alt: false, shift: false }, app_state)?;
      },
      _ => {},
    };
    Ok(None)
  }

  fn handle_events(
    &mut self,
    event: Option<Event>,
    last_tick_key_events: Vec<KeyEvent>,
    app_state: &AppState<'_, DB>,
  ) -> Result<Option<Action>> {
    if app_state.focus != Focus::Editor {
      return Ok(None);
    }
    if let Some(Event::Paste(text)) = event {
      self.textarea.insert_str(text);
    } else if let Some(Event::Mouse(event)) = event {
      self.handle_mouse_events(event, app_state).unwrap();
    } else if let Some(Event::Key(key)) = event {
      let input = Input::from(key);
      self.transition_vim_state(input, app_state)?;
    };
    Ok(None)
  }

  fn update(&mut self, action: Action, app_state: &AppState<'_, DB>) -> Result<Option<Action>> {
    match action {
      Action::MenuPreview(preview_type, schema, table) => {
        if app_state.query_task.is_some() {
          return Ok(None);
        }
        let query = match preview_type {
          MenuPreview::Rows => DB::preview_rows_query(&schema, &table),
          MenuPreview::Columns => DB::preview_columns_query(&schema, &table),
          MenuPreview::Constraints => DB::preview_constraints_query(&schema, &table),
          MenuPreview::Indexes => DB::preview_indexes_query(&schema, &table),
          MenuPreview::Policies => DB::preview_policies_query(&schema, &table),
        };
        self.textarea = TextArea::from(vec![query.clone()]);
        self.textarea.set_search_pattern(keyword_regex()).unwrap();
        self.command_tx.as_ref().unwrap().send(Action::Query(vec![query.clone()], false))?;
      },
      Action::SubmitEditorQuery => {
        if let Some(sender) = &self.command_tx {
          sender.send(Action::Query(self.textarea.lines().to_vec(), false))?;
        }
      },
      Action::HistoryToEditor(lines) => {
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

  fn draw(&mut self, f: &mut Frame<'_>, area: Rect, app_state: &AppState<'_, DB>) -> Result<()> {
    let focused = app_state.focus == Focus::Editor;

    if let Some(query_start) = app_state.last_query_start {
      self.last_query_duration = match app_state.last_query_end {
        Some(end) => Some(end.signed_duration_since(query_start)),
        None => Some(chrono::Utc::now().signed_duration_since(query_start)),
      };
    }

    let duration_string = self.last_query_duration.map_or("".to_string(), |d| {
      let seconds: f64 = (d.num_milliseconds()
        % std::cmp::max(1, d.num_minutes()).saturating_mul(60).saturating_mul(1000)) as f64
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

    self.textarea.set_cursor_style(self.cursor_style);
    self.textarea.set_block(block);
    self.textarea.set_line_number_style(if focused { Style::default().fg(Color::Yellow) } else { Style::new().dim() });
    self.textarea.set_cursor_line_style(Style::default().not_underlined());
    self.textarea.set_hard_tab_indent(false);
    self.textarea.set_tab_length(2);
    self.textarea.set_search_style(Style::default().fg(Color::Magenta).bold());
    f.render_widget(&self.textarea, area);
    Ok(())
  }
}
