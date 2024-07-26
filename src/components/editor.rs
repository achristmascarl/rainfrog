use std::{
  collections::HashMap,
  sync::{Arc, Mutex},
  time::Duration,
};

use color_eyre::eyre::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use ratatui::{prelude::*, widgets::*};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use tui_textarea::{Input, Key, TextArea};

use super::{Component, Frame};
use crate::{
  action::Action,
  app::{App, AppState},
  config::{Config, KeyBindings},
  focus::Focus,
  tui::Event,
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

#[derive(Default, Debug, Clone)]
pub struct Editor<'a> {
  command_tx: Option<UnboundedSender<Action>>,
  config: Config,
  selection: Option<Selection>,
  textarea: TextArea<'a>,
}

impl<'a> Editor<'a> {
  pub fn new() -> Self {
    Editor { command_tx: None, config: Config::default(), selection: None, textarea: TextArea::default() }
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

  fn handle_events(
    &mut self,
    event: Option<Event>,
    last_tick_key_events: Vec<KeyEvent>,
    app_state: &AppState,
  ) -> Result<Option<Action>> {
    if app_state.focus != Focus::Editor {
      return Ok(None);
    }
    if let Some(Event::Key(key)) = event {
      if app_state.query_task.is_none() {
        self.textarea.input(key);
      }
    };
    Ok(None)
  }

  fn update(&mut self, action: Action, app_state: &AppState) -> Result<Option<Action>> {
    if let Action::MenuSelect(schema, table) = action {
      let query = format!("select * from {}.{} limit 100", schema, table);
      self.textarea = TextArea::from(vec![query.clone()]);
      self.command_tx.as_ref().unwrap().send(Action::Query(query))?;
    } else if let Action::SubmitEditorQuery = action {
      if let Some(sender) = &self.command_tx {
        sender.send(Action::Query(self.textarea.lines().join(" ")))?;
      }
    }
    Ok(None)
  }

  fn draw(&mut self, f: &mut Frame<'_>, area: Rect, app_state: &AppState) -> Result<()> {
    let focused = app_state.focus == Focus::Editor;
    let block = Block::default().title("query").borders(Borders::ALL).border_style(if focused {
      Style::new().green()
    } else {
      Style::new().dim()
    });

    self.textarea.set_block(block);
    self.textarea.set_line_number_style(if focused { Style::default().fg(Color::Yellow) } else { Style::new().dim() });
    self.textarea.set_cursor_line_style(Style::default().not_underlined());
    self.textarea.set_hard_tab_indent(false);
    self.textarea.set_tab_length(2);
    f.render_widget(self.textarea.widget(), area);
    Ok(())
  }
}
