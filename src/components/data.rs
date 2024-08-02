use std::{collections::HashMap, sync::Arc, time::Duration};

use color_eyre::eyre::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{prelude::*, widgets::*};
use serde::{Deserialize, Serialize};
use sqlparser::ast::Statement;
use tokio::sync::{mpsc::UnboundedSender, Mutex};

use super::Frame;
use crate::{
  action::Action,
  app::{App, AppState},
  components::{
    scroll_table::{ScrollDirection, ScrollTable},
    Component,
  },
  config::{Config, KeyBindings},
  database::{get_headers, parse_value, row_to_json, row_to_vec, DbError, Rows},
  focus::Focus,
  tui::Event,
};

#[derive(Default)]
pub enum DataState {
  #[default]
  Blank,
  Loading,
  NoResults,
  HasResults(Rows),
  Error(DbError),
  Cancelled,
  RowsAffected(u64),
  StatementCompleted(Statement),
}

pub trait SettableDataTable<'a> {
  fn set_data_state(&mut self, data: Option<Result<Rows, DbError>>, statement_type: Statement);
  fn set_loading(&mut self);
  fn set_cancelled(&mut self);
}

pub trait DataComponent<'a>: Component + SettableDataTable<'a> {}
impl<'a, T> DataComponent<'a> for T where T: Component + SettableDataTable<'a>
{
}

#[derive(Default)]
pub struct Data<'a> {
  command_tx: Option<UnboundedSender<Action>>,
  config: Config,
  scrollable: ScrollTable<'a>,
  data_state: DataState,
}

impl<'a> Data<'a> {
  pub fn new() -> Self {
    Data {
      command_tx: None,
      config: Config::default(),
      scrollable: ScrollTable::default(),
      data_state: DataState::Blank,
    }
  }
}

impl<'a> SettableDataTable<'a> for Data<'a> {
  fn set_data_state(&mut self, data: Option<Result<Rows, DbError>>, statement_type: Statement) {
    match data {
      Some(Ok(rows)) => {
        if rows.0.is_empty() && rows.1.is_some_and(|n| n > 0) {
          self.data_state = DataState::RowsAffected(rows.1.unwrap());
        } else if rows.0.is_empty() && !matches!(statement_type, Statement::Query(_)) {
          self.data_state = DataState::StatementCompleted(statement_type);
        } else if rows.0.is_empty() {
          self.data_state = DataState::NoResults;
        } else {
          let headers = get_headers(&rows);
          let header_row =
            Row::new(headers.iter().map(|h| Cell::from(format!("{}\n{}", h.name, h.type_name))).collect::<Vec<Cell>>())
              .height(2)
              .bottom_margin(1);
          let value_rows = rows.0.iter().map(|r| Row::new(row_to_vec(r)).bottom_margin(1)).collect::<Vec<Row>>();
          let buf_table =
            Table::default().rows(value_rows).header(header_row).style(Style::default()).column_spacing(1);
          self.scrollable.set_table(Box::new(buf_table), headers.len(), rows.0.len(), 36_u16);
          self.data_state = DataState::HasResults(rows);
        }
      },
      Some(Err(e)) => {
        self.data_state = DataState::Error(e);
      },
      _ => {
        self.data_state = DataState::Blank;
      },
    }
  }

  fn set_loading(&mut self) {
    self.data_state = DataState::Loading;
  }

  fn set_cancelled(&mut self) {
    self.data_state = DataState::Cancelled;
  }
}

impl<'a> Component for Data<'a> {
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
    if app_state.focus != Focus::Data {
      return Ok(None);
    }
    if let Some(Event::Key(key)) = event {
      match key.code {
        KeyCode::Right | KeyCode::Char('l') => {
          self.scrollable.scroll(ScrollDirection::Right);
        },
        KeyCode::Left | KeyCode::Char('h') => {
          self.scrollable.scroll(ScrollDirection::Left);
        },
        KeyCode::Down | KeyCode::Char('j') => {
          self.scrollable.scroll(ScrollDirection::Down);
        },
        KeyCode::Up | KeyCode::Char('k') => {
          self.scrollable.scroll(ScrollDirection::Up);
        },
        KeyCode::Char('e') => {
          self.scrollable.next_column();
        },
        KeyCode::Char('b') => {
          self.scrollable.prev_column();
        },
        KeyCode::Char('g') => {
          self.scrollable.top_row();
        },
        KeyCode::Char('G') => {
          self.scrollable.bottom_row();
        },
        KeyCode::Char('0') => {
          self.scrollable.first_column();
        },
        KeyCode::Char('$') => {
          self.scrollable.last_column();
        },
        _ => {},
      }
    };
    Ok(None)
  }

  fn update(&mut self, action: Action, app_state: &AppState) -> Result<Option<Action>> {
    if let Action::Query(query) = action {
      self.scrollable.reset_scroll();
    }
    Ok(None)
  }

  fn draw(&mut self, f: &mut Frame<'_>, area: Rect, app_state: &AppState) -> Result<()> {
    let focused = app_state.focus == Focus::Data;

    let mut block = Block::default().borders(Borders::ALL).border_style(if focused {
      Style::new().green()
    } else {
      Style::new().dim()
    });

    if let DataState::HasResults(rows) = &self.data_state {
      block = block.title(format!("results ({} rows)", rows.0.len()));
    } else {
      block = block.title("results");
    }

    match &self.data_state {
      DataState::NoResults => {
        f.render_widget(Paragraph::new("no results").wrap(Wrap { trim: false }).block(block), area);
      },
      DataState::StatementCompleted(statement) => {
        f.render_widget(
          Paragraph::new(format!(
            "{} statement completed",
            format!("{:?}", statement).split('(').collect::<Vec<&str>>()[0].split('{').collect::<Vec<&str>>()[0]
              .split('[')
              .collect::<Vec<&str>>()[0]
              .trim()
          ))
          .wrap(Wrap { trim: false })
          .block(block),
          area,
        );
      },
      DataState::RowsAffected(n) => {
        f.render_widget(
          Paragraph::new(format!("{} row{} affected", n, if *n == 1_u64 { "" } else { "s" }))
            .wrap(Wrap { trim: false })
            .block(block),
          area,
        );
      },
      DataState::Blank => {
        f.render_widget(Paragraph::new("").wrap(Wrap { trim: false }).block(block), area);
      },
      DataState::HasResults(rows) => {
        self.scrollable.block(block);
        self.scrollable.draw(f, area, app_state)?;
      },
      DataState::Error(e) => {
        f.render_widget(
          Paragraph::new(e.to_string()).style(Style::default().fg(Color::Red)).wrap(Wrap { trim: false }).block(block),
          area,
        );
      },
      DataState::Loading => {
        f.render_widget(
          Paragraph::new(Text::from("loading...").fg(Color::Green)).wrap(Wrap { trim: false }).block(block),
          area,
        );
      },
      DataState::Cancelled => {
        f.render_widget(
          Paragraph::new(Text::from("query cancelled.").fg(Color::Yellow)).wrap(Wrap { trim: false }).block(block),
          area,
        );
      },
    }

    Ok(())
  }
}

// // TODO: see if this trait can be fixed and used
//
// // based on: https://users.rust-lang.org/t/casting-traitobject-to-super-trait/33524/9
// pub trait IntoComponent<'a, Super: ?Sized> {
//   fn as_super(&self) -> &Super;
//   fn as_super_mut(&mut self) -> &mut Super;
//   fn into_super(self: Box<Self>) -> Box<Super>;
//   fn into_super_ref_mut(self: &'a mut Box<Self>) -> &'a mut Box<Super>;
// }
//
// impl<'a, T: 'a + Component> IntoComponent<'a, dyn Component + 'a> for T
// where
//   T: Component + 'a,
// {
//   fn as_super(&self) -> &(dyn Component + 'a) {
//     self
//   }
//
//   fn as_super_mut(&mut self) -> &mut (dyn Component + 'a) {
//     self
//   }
//
//   fn into_super(self: Box<Self>) -> Box<dyn Component + 'a> {
//     self
//   }
//
//   fn into_super_ref_mut(self: &'a mut Box<Self>) -> &'a mut Box<dyn Component + 'a> {
//     self
//   }
// }
//
