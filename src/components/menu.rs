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
  database::{get_headers, parse_value, row_to_json, row_to_vec, DbError, Rows},
  focus::Focus,
};

pub trait SettableTableList<'a> {
  fn set_table_list(&mut self, data: Option<Result<Rows, DbError>>);
}

pub trait MenuComponent<'a>: Component + SettableTableList<'a> {}
impl<'a, T> MenuComponent<'a> for T where T: Component + SettableTableList<'a>
{
}

pub struct Menu {
  command_tx: Option<UnboundedSender<Action>>,
  config: Config,
}

impl Menu {
  pub fn new() -> Self {
    Menu { command_tx: None, config: Config::default() }
  }
}

impl<'a> SettableTableList<'a> for Menu {
  fn set_table_list(&mut self, data: Option<Result<Rows, DbError>>) {
    log::info!("setting menu table list");
    match data {
      Some(Ok(rows)) => {
        rows.iter().for_each(|row| log::info!("{}", row_to_vec(row).join(",")));
      },
      Some(Err(e)) => {
        log::info!("{}", e);
      },
      None => {},
    }
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

  fn draw(&mut self, f: &mut Frame<'_>, area: Rect, app_state: &AppState) -> Result<()> {
    let focused = app_state.focus == Focus::Menu;

    f.render_widget(
      Block::default().title(app_state.connection_string.to_string()).borders(Borders::ALL).border_style(if focused {
        Style::new().green()
      } else {
        Style::new().dim()
      }),
      area,
    );
    Ok(())
  }
}
