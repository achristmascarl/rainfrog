use std::{
  collections::HashMap,
  sync::{Arc, Mutex},
  time::Duration,
};

use color_eyre::eyre::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{prelude::*, widgets::*};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

use super::{Component, Frame};
use crate::{
  action::Action,
  app::{App, AppState},
  config::{Config, KeyBindings},
  focus::Focus,
};

pub struct Data {
  command_tx: Option<UnboundedSender<Action>>,
  config: Config,
  state: Arc<Mutex<AppState>>,
}

impl Data {
  pub fn new(state: Arc<Mutex<AppState>>) -> Self {
    Data { command_tx: None, config: Config::default(), state }
  }
}

impl Component for Data {
  fn register_action_handler(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
    self.command_tx = Some(tx);
    Ok(())
  }

  fn register_config_handler(&mut self, config: Config) -> Result<()> {
    self.config = config;
    Ok(())
  }

  fn draw(&mut self, f: &mut Frame<'_>, area: Rect) -> Result<()> {
    let state = self.state.lock().unwrap();
    let focused = state.focus == Focus::Data;

    f.render_widget(
      Block::default().title("bottom").borders(Borders::ALL).border_style(if focused {
        Style::new().green()
      } else {
        Style::new().dim()
      }),
      area,
    );

    Ok(())
  }
}
