use std::{collections::HashMap, sync::Arc, time::Duration};

use color_eyre::eyre::Result;
use crossterm::{
  event::{KeyCode, KeyEvent, MouseEventKind},
  terminal::ScrollDown,
};
use ratatui::{prelude::*, widgets::*};
use serde::{Deserialize, Serialize};
use sqlparser::ast::Statement;
use tokio::sync::{mpsc::UnboundedSender, Mutex};

use super::{scroll_table::SelectionMode, Frame};
use crate::{
  action::Action,
  app::{App, AppState},
  components::{
    scroll_table::{ScrollDirection, ScrollTable},
    Component,
  },
  config::{Config, KeyBindings},
  database::{get_headers, parse_value, row_to_json, row_to_vec, statement_type_string, DbError, Rows},
  focus::Focus,
  tui::Event,
};

#[derive(Default)]
pub enum DataState<'a> {
  #[default]
  Blank,
  Loading,
  NoResults,
  HasResults(Rows),
  Explain(Text<'a>),
  Error(DbError),
  Cancelled,
  RowsAffected(u64),
  StatementCompleted(Statement),
}

pub struct ParagraphScroll {
  pub y_offset: u16,
  pub x_offset: u16,
}

pub trait SettableDataTable<'a> {
  fn set_data_state(&mut self, data: Option<Result<Rows, DbError>>, statement_type: Option<Statement>);
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
  data_state: DataState<'a>,
  paragraph_scroll: Option<ParagraphScroll>,
  paragraph_width: u16,
  paragraph_height: u16,
}

impl<'a> Data<'a> {
  pub fn new() -> Self {
    Data {
      command_tx: None,
      config: Config::default(),
      scrollable: ScrollTable::default(),
      data_state: DataState::Blank,
      paragraph_scroll: None,
      paragraph_width: 0,
      paragraph_height: 0,
    }
  }
}

impl<'a> SettableDataTable<'a> for Data<'a> {
  fn set_data_state(&mut self, data: Option<Result<Rows, DbError>>, statement_type: Option<Statement>) {
    match data {
      Some(Ok(rows)) => {
        if rows.rows.is_empty() && rows.rows_affected.is_some_and(|n| n > 0) {
          self.data_state = DataState::RowsAffected(rows.rows_affected.unwrap());
        } else if rows.rows.is_empty()
          && statement_type.is_some()
          && !matches!(statement_type, Some(Statement::Query(_)))
        {
          self.data_state = DataState::StatementCompleted(statement_type.unwrap());
        } else if rows.rows.is_empty() {
          self.data_state = DataState::NoResults;
        } else if matches!(statement_type, Some(Statement::Explain { .. })) {
          self.data_state = DataState::Explain(Text::from_iter(rows.rows.iter().map(|r| r.join(" "))));
        } else {
          let header_row = Row::new(
            rows.headers.iter().map(|h| Cell::from(format!("{}\n{}", h.name, h.type_name))).collect::<Vec<Cell>>(),
          )
          .height(2)
          .bottom_margin(1);
          let value_rows = rows.rows.iter().map(|r| Row::new(r.clone()).bottom_margin(1));
          let buf_table = Table::default()
            .rows(value_rows)
            .header(header_row)
            .style(Style::default())
            .column_spacing(1)
            .highlight_style(Style::default().fg(Color::LightBlue).reversed().bold());
          self.scrollable.set_table(buf_table, rows.headers.len(), rows.rows.len(), 36_u16);
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

  fn handle_mouse_events(
    &mut self,
    mouse: crossterm::event::MouseEvent,
    app_state: &AppState,
  ) -> Result<Option<Action>> {
    if app_state.focus != Focus::Data {
      return Ok(None);
    }
    match mouse.kind {
      MouseEventKind::ScrollDown => {
        self.scrollable.scroll(ScrollDirection::Down);
      },
      MouseEventKind::ScrollUp => {
        self.scrollable.scroll(ScrollDirection::Up);
      },
      MouseEventKind::ScrollLeft => {
        self.scrollable.scroll(ScrollDirection::Left);
      },
      MouseEventKind::ScrollRight => {
        self.scrollable.scroll(ScrollDirection::Right);
      },
      _ => {},
    };
    Ok(None)
  }

  fn handle_key_events(&mut self, key: KeyEvent, app_state: &AppState) -> Result<Option<Action>> {
    if app_state.focus != Focus::Data {
      return Ok(None);
    }
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
      KeyCode::Char('e') | KeyCode::Char('w') => {
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
      KeyCode::Char('v') => {
        self.scrollable.transition_selection_mode(Some(SelectionMode::Cell));
      },
      KeyCode::Char('V') => {
        self.scrollable.transition_selection_mode(Some(SelectionMode::Row));
      },
      KeyCode::Enter => {
        match self.scrollable.get_selection_mode() {
          Some(SelectionMode::Row) => {
            self.scrollable.transition_selection_mode(Some(SelectionMode::Cell));
          },
          None | Some(SelectionMode::Copied) => {
            self.scrollable.transition_selection_mode(Some(SelectionMode::Row));
          },
          _ => {},
        };
      },
      KeyCode::Backspace => {
        match self.scrollable.get_selection_mode() {
          Some(SelectionMode::Row) => {
            self.scrollable.transition_selection_mode(None);
          },
          Some(SelectionMode::Cell) => {
            self.scrollable.transition_selection_mode(Some(SelectionMode::Row));
          },
          _ => {},
        };
      },
      KeyCode::Char('y') => {
        if let DataState::HasResults(Rows { rows, .. }) = &self.data_state {
          let (x, y) = self.scrollable.get_cell_offsets();
          let row = &rows[y];
          match self.scrollable.get_selection_mode() {
            Some(SelectionMode::Row) => {
              let row_string = row.join(", ");
              self.command_tx.clone().unwrap().send(Action::CopyData(row_string))?;
              self.scrollable.transition_selection_mode(Some(SelectionMode::Copied));
            },
            Some(SelectionMode::Cell) => {
              let cell = row[x as usize].clone();
              self.command_tx.clone().unwrap().send(Action::CopyData(cell))?;
              self.scrollable.transition_selection_mode(Some(SelectionMode::Copied));
            },
            _ => {},
          }
        }
      },
      KeyCode::Esc => {
        self.scrollable.transition_selection_mode(None);
      },
      _ => {},
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

    if let DataState::HasResults(Rows { rows, .. }) = &self.data_state {
      let (x, y) = self.scrollable.get_cell_offsets();
      let row = &rows[y];
      let title_string = match self.scrollable.get_selection_mode() {
        Some(SelectionMode::Row) => {
          format!(" 󰆼 results <alt+4> (row {} of {})", y.saturating_add(1), rows.len())
        },
        Some(SelectionMode::Cell) => {
          format!(" 󰆼 results <alt+4> (row {} of {}) - {} ", y.saturating_add(1), rows.len(), row[x as usize].clone())
        },
        Some(SelectionMode::Copied) => {
          format!(" 󰆼 results <alt+4> ({} rows) - copied! ", rows.len())
        },
        _ => format!(" 󰆼 results <alt+4> ({} rows)", rows.len()),
      };
      block = block.title(title_string);
    } else {
      block = block.title(" 󰆼 results <alt+4>");
    }

    match &self.data_state {
      DataState::NoResults => {
        f.render_widget(Paragraph::new("no results").wrap(Wrap { trim: false }).block(block), area);
      },
      DataState::StatementCompleted(statement) => {
        f.render_widget(
          Paragraph::new(format!("{} statement completed", statement_type_string(statement)))
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
      DataState::Explain(text) => {
        let paragraph = Paragraph::new(text.clone()).block(block);
        f.render_widget(paragraph, area)
      },
      DataState::HasResults(_) => {
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
