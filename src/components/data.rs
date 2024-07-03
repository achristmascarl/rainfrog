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
  widgets::scrollable::{Scrollable, ScrollableState},
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
    let mut state = self.state.lock().unwrap();
    let focused = state.focus == Focus::Data;

    let block = Block::default().title("bottom").borders(Borders::ALL).border_style(if focused {
      Style::new().green()
    } else {
      Style::new().dim()
    });

    match &state.data {
      Some(Ok(rows)) => 'rows: {
        if rows.is_empty() {
          f.render_widget(Paragraph::new("no results").wrap(Wrap { trim: false }).block(block), area);
          break 'rows;
        }
        let headers = get_headers(rows);
        let header_row =
          Row::new(headers.iter().map(|h| Cell::from(format!("{}\n{}", h.name, h.type_name))).collect::<Vec<Cell>>())
            .height(2)
            .bottom_margin(1);
        let value_rows = rows.iter().map(|r| Row::new(row_to_vec(r)).bottom_margin(1)).collect::<Vec<Row>>();
        let buf_table = Table::default().rows(value_rows).header(header_row).style(Style::default()).column_spacing(1);
        let max_height = 250;
        let max_width = 500;
        let scrollable = Scrollable::new(Box::new(buf_table.clone()), max_height, max_width).block(block);
        let mut scrollable_state = ScrollableState::default();

        if !state.table_buf_logged {
          scrollable.log();
          state.table_buf_logged = true;
        }

        f.render_stateful_widget(scrollable, area, &mut scrollable_state);
      },
      Some(Err(e)) => {
        f.render_widget(Paragraph::new(format!("{:?}", e.to_string())).wrap(Wrap { trim: false }).block(block), area)
      },
      _ => f.render_widget(Paragraph::new("").wrap(Wrap { trim: false }).block(block), area),
    }

    Ok(())
  }
}
