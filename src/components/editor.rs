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
pub struct Editor {
  command_tx: Option<UnboundedSender<Action>>,
  config: Config,
  lines: Vec<Vec<char>>,
  cursor: CursorPosition,
  selection: Option<Selection>,
}

impl Editor {
  pub fn new() -> Self {
    Editor {
      command_tx: None,
      config: Config::default(),
      cursor: CursorPosition { row: 0, col: 0 },
      selection: None,
      lines: vec![vec![]],
    }
  }
}

impl Component for Editor {
  fn register_action_handler(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
    self.command_tx = Some(tx);
    Ok(())
  }

  fn register_config_handler(&mut self, config: Config) -> Result<()> {
    self.config = config;
    Ok(())
  }

  fn handle_events(&mut self, event: Option<Event>, app_state: &AppState) -> Result<Option<Action>> {
    if app_state.focus != Focus::Editor {
      return Ok(None);
    }
    if let Some(Event::Key(key)) = event {
      if app_state.query_task.is_none() {
        match key.code {
          KeyCode::Enter => {
            if let Some(sender) = &self.command_tx {
              sender.send(Action::Query(self.lines[0].iter().collect::<String>()))?;
            }
          },
          KeyCode::Backspace => {
            if !self.lines[0].is_empty() {
              self.lines[0].pop();
            };
          },
          KeyCode::Char(c) => {
            self.lines[0].push(c);
          },
          _ => {},
        }
      }
    };
    Ok(None)
  }

  fn update(&mut self, action: Action, app_state: &AppState) -> Result<Option<Action>> {
    if let Action::MenuSelect(schema, table) = action {
      let query = format!("select * from {}.{} limit 100", schema, table);
      let chars: Vec<char> = format!("select * from {}.{} limit 100", schema, table).chars().collect();
      self.lines = vec![chars];
      self.command_tx.as_ref().unwrap().send(Action::Query(query))?;
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
    let text = Paragraph::new(self.lines[0].iter().collect::<String>()).wrap(Wrap { trim: false }).block(block);

    f.render_widget(text, area);
    Ok(())
  }
}
