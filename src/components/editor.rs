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
use tokio::sync::mpsc::UnboundedSender;
use tui_textarea::{Input, Key, Scrolling, TextArea};

use super::{Component, Frame};
use crate::{
  action::{Action, MenuPreview},
  app::{App, AppState},
  config::{Config, KeyBindings},
  database::get_keywords,
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
  format!("(?i)(^|[^a-zA-Z0-9\'\"._]+)({})($|[^a-zA-Z0-9\'\"._]+)", get_keywords().join("|"))
}

#[derive(Default)]
pub struct Editor<'a> {
  command_tx: Option<UnboundedSender<Action>>,
  config: Config,
  selection: Option<Selection>,
  textarea: TextArea<'a>,
  vim_state: Vim,
  cursor_style: Style,
}

impl<'a> Editor<'a> {
  pub fn new() -> Self {
    let mut textarea = TextArea::default();
    textarea.set_search_pattern(keyword_regex()).unwrap();
    textarea.set_search_style(Style::default().fg(Color::Magenta).bold());

    Editor {
      command_tx: None,
      config: Config::default(),
      selection: None,
      textarea,
      vim_state: Vim::new(Mode::Normal),
      cursor_style: Mode::Normal.cursor_style(),
    }
  }

  pub fn transition_vim_state(&mut self, input: Input) -> Result<()> {
    match input {
      Input { key: Key::Enter, alt: true, .. } => {
        if let Some(sender) = &self.command_tx {
          sender.send(Action::Query(self.textarea.lines().join(" ")))?;
          self.vim_state = Vim::new(Mode::Normal);
          self.cursor_style = Mode::Normal.cursor_style();
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
      },
    };
    Ok(())
  }
}

impl<'a> Component for Editor<'a> {
  fn register_action_handler(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
    self.command_tx = Some(tx);
    Ok(())
  }

  fn register_config_handler(&mut self, config: Config) -> Result<()> {
    self.config = config;
    Ok(())
  }

  fn handle_mouse_events(&mut self, mouse: MouseEvent, app_state: &AppState) -> Result<Option<Action>> {
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
        self.transition_vim_state(Input { key: Key::Char('h'), ctrl: false, alt: false, shift: false })?;
      },
      MouseEventKind::ScrollRight => {
        self.transition_vim_state(Input { key: Key::Char('j'), ctrl: false, alt: false, shift: false })?;
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
    if let Some(Event::Paste(text)) = event {
      self.textarea.insert_str(text);
    } else if let Some(Event::Mouse(event)) = event {
      self.handle_mouse_events(event, app_state).unwrap();
    } else if let Some(Event::Key(key)) = event {
      if app_state.query_task.is_none() {
        let input = Input::from(key);
        self.transition_vim_state(input)?;
      }
    };
    Ok(None)
  }

  fn update(&mut self, action: Action, app_state: &AppState) -> Result<Option<Action>> {
    if let Action::MenuPreview(preview_type, schema, table) = action {
      if app_state.query_task.is_some() {
        return Ok(None);
      }
      let query = match preview_type {
        MenuPreview::Rows => format!("select * from {}.{} limit 100", schema, table),
        MenuPreview::Columns => {
          format!(
            "select column_name, * from information_schema.columns where table_schema = '{}' and table_name = '{}'",
            schema, table
          )
        },
        MenuPreview::Constraints => {
          format!(
            "select constraint_name, * from information_schema.table_constraints where table_schema = '{}' and table_name = '{}'",
            schema, table
          )
        },
        MenuPreview::Indexes => {
          format!(
            "select indexname, indexdef, * from pg_indexes where schemaname = '{}' and tablename = '{}'",
            schema, table
          )
        },
        MenuPreview::Policies => {
          format!("select * from pg_policies where schemaname = '{}' and tablename = '{}'", schema, table)
        },
      };
      self.textarea = TextArea::from(vec![query.clone()]);
      self.textarea.set_search_pattern(keyword_regex()).unwrap();
      self.textarea.set_search_style(Style::default().fg(Color::Magenta).bold());
      self.command_tx.as_ref().unwrap().send(Action::Query(query))?;
    } else if let Action::SubmitEditorQuery = action {
      if let Some(sender) = &self.command_tx {
        sender.send(Action::Query(self.textarea.lines().join(" ")))?;
      }
    } else if let Action::CopyData(data) = action {
      #[cfg(not(feature = "termux"))]
      {
        let mut clipboard = Clipboard::new().unwrap();
        clipboard.set_text(data).unwrap();
      }
      #[cfg(feature = "termux")]
      {
        self.textarea.set_yank_text(data);
      }
    }
    Ok(None)
  }

  fn draw(&mut self, f: &mut Frame<'_>, area: Rect, app_state: &AppState) -> Result<()> {
    let focused = app_state.focus == Focus::Editor;
    let block = self.vim_state.mode.block().title(" 󰤏 query <alt+2> ").border_style(if focused {
      Style::new().green()
    } else {
      Style::new().dim()
    });

    self.textarea.set_cursor_style(self.cursor_style);
    self.textarea.set_block(block);
    self.textarea.set_line_number_style(if focused { Style::default().fg(Color::Yellow) } else { Style::new().dim() });
    self.textarea.set_cursor_line_style(Style::default().not_underlined());
    self.textarea.set_hard_tab_indent(false);
    self.textarea.set_tab_length(2);
    f.render_widget(&self.textarea, area);
    Ok(())
  }
}
