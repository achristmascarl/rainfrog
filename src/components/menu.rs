use std::{collections::HashMap, sync::Arc, time::Duration};

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
};

pub struct Menu {
  command_tx: Option<UnboundedSender<Action>>,
  config: Config,
  state: Arc<AppState>,
}

impl Menu {
  pub fn new(state: Arc<AppState>) -> Self {
    Menu { command_tx: None, config: Config::default(), state }
  }
}

impl Component for Menu {
  fn register_action_handler(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
    self.command_tx = Some(tx);
    Ok(())
  }

  fn register_config_handler(&mut self, config: Config) -> Result<()> {
    self.config = config;
    Ok(())
  }

  fn update(&mut self, action: Action) -> Result<Option<Action>> {
    match action {
      Action::Tick => {},
      _ => {},
    }
    Ok(None)
  }

  fn draw(&mut self, f: &mut Frame<'_>, area: Rect) -> Result<()> {
    let state = self.state.clone();

    f.render_widget(Block::default().title(state.connection_string.to_string()).borders(Borders::ALL), area);
    Ok(())
  }
}
