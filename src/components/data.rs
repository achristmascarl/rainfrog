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

        if !state.table_buf_logged {
          let max_height = 250;
          let max_width = 500;
          let mut buf = Buffer::empty(Rect::new(0, 0, max_width, max_height));
          ratatui::widgets::Widget::render(buf_table.clone(), buf.area, &mut buf);
          buf = clamp(buf);
          let buf_height = buf.area.height;
          let buf_width = buf.area.width;
          for n in 0..buf_height {
            let mut line: String = String::from("");
            let cells = buf.content.to_vec()[((n * buf_width) as usize)..(((n + 1) * buf_width) as usize)].to_vec();
            for cell in cells.iter() {
              line += cell.clone().symbol();
            }
            // log::info!(
            //   "rendering line {}/{}, length {}, last symbol {}, last symbol is blank {}:",
            //   n,
            //   buf_height,
            //   line.len(),
            //   cells[cells.len() - 1].symbol(),
            //   cells[cells.len() - 1].symbol() == " "
            // );
            log::info!("{}", line.as_str());
          }
          state.table_buf_logged = true;
        }

        let table = buf_table.block(block);
        f.render_widget(table, area);
      },
      Some(Err(e)) => {
        f.render_widget(Paragraph::new(format!("{:?}", e.to_string())).wrap(Wrap { trim: false }).block(block), area)
      },
      _ => f.render_widget(Paragraph::new("").wrap(Wrap { trim: false }).block(block), area),
    }

    Ok(())
  }
}

pub fn clamp(buf: Buffer) -> Buffer {
  let height = buf.area.height;
  let width = buf.area.width;
  log::info!("original height: {}, width: {}", height, width);
  let mut used_height: u16 = 0;
  let mut used_width: u16 = 0;
  for i in (0..height).rev() {
    let row = buf.content.to_vec()[((i * width) as usize)..(((i + 1) * width) as usize)].to_vec();
    for j in (0..width).rev() {
      let cell = row[j as usize].clone();
      if cell.symbol() != " " {
        used_height = std::cmp::max(used_height, i + 1);
        used_width = std::cmp::max(used_width, j + 1);
      }
    }
  }
  let mut content: Vec<ratatui::buffer::Cell> = Vec::new();
  for i in 0..used_height {
    let row = buf.content.to_vec()[((i * width) as usize)..(((i + 1) * width) as usize)].to_vec();
    for j in 0..used_width {
      content.push(row[j as usize].clone().to_owned());
    }
  }
  log::info!("clamped height: {}, width: {}", used_height, used_width);
  Buffer { area: Rect::new(0, 0, used_width, used_height), content }
}
