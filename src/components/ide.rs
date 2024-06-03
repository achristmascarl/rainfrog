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

struct CursorPosition {
  pub row: u32,
  pub line: u32,
}

struct Selection {
  pub start: CursorPosition,
  pub end: CursorPosition,
}

pub struct IDE {
  command_tx: Option<UnboundedSender<Action>>,
  config: Config,
  state: Arc<Mutex<AppState>>,
  lines: Vec<Vec<char>>,
  cursor: CursorPosition,
  selection: Option<Selection>,
}

impl IDE {
  pub fn new(state: Arc<Mutex<AppState>>) -> Self {
    IDE {
      command_tx: None,
      config: Config::default(),
      state,
      cursor: CursorPosition { row: 0, line: 0 },
      selection: None,
      lines: vec![vec![]],
    }
  }
}

impl Component for IDE {
  fn register_action_handler(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
    self.command_tx = Some(tx);
    Ok(())
  }

  fn register_config_handler(&mut self, config: Config) -> Result<()> {
    self.config = config;
    Ok(())
  }

  fn handle_events(&mut self, event: Option<Event>) -> Result<Option<Action>> {
    let state = self.state.lock().unwrap();
    if state.focus != Focus::IDE {
      return Ok(None);
    }
    if let Some(Event::Key(key)) = event {
      match key.code {
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
    };
    Ok(None)
  }

  fn draw(&mut self, f: &mut Frame<'_>, area: Rect) -> Result<()> {
    let state = self.state.lock().unwrap();
    let focused = state.focus == Focus::IDE;
    let block = Block::default().title("top").borders(Borders::ALL).border_style(if focused {
      Style::new().green()
    } else {
      Style::new().dim()
    });
    let text = Paragraph::new(self.lines[0].iter().collect::<String>()).block(block);

    f.render_widget(text, area);
    Ok(())
  }
}
