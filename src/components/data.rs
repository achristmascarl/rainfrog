use std::collections::VecDeque;
use std::io::{self, Write};
use std::process::{Command, Stdio};

use color_eyre::eyre::{self, Result};
use crossterm::event::{KeyEvent, MouseEventKind};
use csv::Writer;
use ratatui::{prelude::*, symbols::scrollbar, widgets::*};
use sqlparser::ast::Statement;
use tokio::sync::mpsc::UnboundedSender;
use tui_textarea::{Input, Key};

use super::{Frame, scroll_table::SelectionMode};
use crate::{
  action::Action,
  app::AppState,
  components::{
    Component,
    scroll_table::{ScrollDirection, ScrollTable},
  },
  config::Config,
  database::{Rows, header_to_vec, statement_type_string},
  focus::Focus,
  utils::get_export_dir,
};

#[allow(clippy::large_enum_variant)]
#[derive(Default)]
pub enum DataState<'a> {
  #[default]
  Blank,
  Loading,
  NoResults,
  HasResults(Rows),
  Explain(Text<'a>),
  Error(eyre::Report),
  Cancelled,
  RowsAffected(u64),
  StatementCompleted(Statement),
}

#[derive(Clone, Debug)]
pub struct ExplainOffsets {
  pub y_offset: u16,
  pub x_offset: u16,
}

pub trait SettableDataTable<'a> {
  fn set_data_state(&mut self, data: Option<Result<Rows>>, statement_type: Option<Statement>);
  fn set_loading(&mut self);
  fn set_cancelled(&mut self);
}

pub trait DataComponent<'a>: Component + SettableDataTable<'a> {}
impl<'a, T> DataComponent<'a> for T where T: Component + SettableDataTable<'a> {}

#[derive(Default)]
pub struct Data<'a> {
  command_tx: Option<UnboundedSender<Action>>,
  config: Config,
  scrollable: ScrollTable<'a>,
  data_state: DataState<'a>,
  explain_scroll: Option<ExplainOffsets>,
  explain_width: u16,
  explain_height: u16,
  explain_max_x_offset: u16,
  explain_max_y_offset: u16,
}

impl Data<'_> {
  pub fn new() -> Self {
    Data {
      command_tx: None,
      config: Config::default(),
      scrollable: ScrollTable::default(),
      data_state: DataState::Blank,
      explain_scroll: None,
      explain_width: 0,
      explain_height: 0,
      explain_max_x_offset: 0,
      explain_max_y_offset: 0,
    }
  }

  pub fn scroll(&mut self, direction: ScrollDirection) {
    if let DataState::Explain(_) = self.data_state {
      if let Some(offsets) = self.explain_scroll.clone() {
        match direction {
          ScrollDirection::Up => {
            self.explain_scroll =
              Some(ExplainOffsets { y_offset: offsets.y_offset.saturating_sub(1), x_offset: offsets.x_offset });
          },
          ScrollDirection::Down => {
            self.explain_scroll =
              Some(ExplainOffsets { y_offset: offsets.y_offset.saturating_add(1), x_offset: offsets.x_offset });
          },
          ScrollDirection::Left => {
            self.explain_scroll =
              Some(ExplainOffsets { y_offset: offsets.y_offset, x_offset: offsets.x_offset.saturating_sub(2) });
          },
          ScrollDirection::Right => {
            self.explain_scroll =
              Some(ExplainOffsets { y_offset: offsets.y_offset, x_offset: offsets.x_offset.saturating_add(2) });
          },
        };
      }
    } else if let DataState::HasResults(_) = self.data_state {
      self.scrollable.scroll(direction);
    }
  }

  pub fn top(&mut self) {
    if let DataState::Explain(_) = self.data_state {
      match self.explain_scroll {
        Some(ExplainOffsets { x_offset, .. }) => {
          self.explain_scroll = Some(ExplainOffsets { y_offset: 0, x_offset });
        },
        _ => {
          self.explain_scroll = Some(ExplainOffsets { y_offset: 0, x_offset: 0 });
        },
      }
    } else if let DataState::HasResults(_) = self.data_state {
      self.scrollable.top_row();
    }
  }

  pub fn bottom(&mut self) {
    if let DataState::Explain(_) = self.data_state {
      match self.explain_scroll {
        Some(ExplainOffsets { x_offset, .. }) => {
          self.explain_scroll = Some(ExplainOffsets { y_offset: self.explain_max_y_offset, x_offset });
        },
        _ => {
          self.explain_scroll = Some(ExplainOffsets { y_offset: self.explain_max_y_offset, x_offset: 0 });
        },
      }
    } else if let DataState::HasResults(_) = self.data_state {
      self.scrollable.bottom_row();
    }
  }

  pub fn left(&mut self) {
    if let DataState::Explain(_) = self.data_state {
      match self.explain_scroll {
        Some(ExplainOffsets { y_offset, .. }) => {
          self.explain_scroll = Some(ExplainOffsets { y_offset, x_offset: 0 });
        },
        _ => {
          self.explain_scroll = Some(ExplainOffsets { y_offset: 0, x_offset: 0 });
        },
      }
    } else if let DataState::HasResults(_) = self.data_state {
      self.scrollable.first_column();
    }
  }

  pub fn right(&mut self) {
    if let DataState::Explain(_) = self.data_state {
      match self.explain_scroll {
        Some(ExplainOffsets { y_offset, .. }) => {
          self.explain_scroll = Some(ExplainOffsets { y_offset, x_offset: self.explain_max_x_offset });
        },
        _ => {
          self.explain_scroll = Some(ExplainOffsets { y_offset: 0, x_offset: self.explain_max_x_offset });
        },
      }
    } else if let DataState::HasResults(_) = self.data_state {
      self.scrollable.last_column();
    }
  }
}

impl<'a> SettableDataTable<'a> for Data<'a> {
  fn set_data_state(&mut self, data: Option<Result<Rows>>, statement_type: Option<Statement>) {
    self.explain_width = 0;
    self.explain_height = 0;
    self.explain_max_x_offset = 0;
    self.explain_max_y_offset = 0;
    self.explain_scroll = None;
    self.scrollable = ScrollTable::default();
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
          self.explain_width = rows.rows.iter().fold(0_u16, |acc, r| acc.max(r.join(" ").len() as u16));
          self.explain_height = rows.rows.len() as u16;
          self.explain_scroll = Some(ExplainOffsets { y_offset: 0, x_offset: 0 });
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
            .row_highlight_style(Style::default().fg(Color::LightBlue).reversed().bold());
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

impl Component for Data<'_> {
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
        self.scroll(ScrollDirection::Down);
      },
      MouseEventKind::ScrollUp => {
        self.scroll(ScrollDirection::Up);
      },
      MouseEventKind::ScrollLeft => {
        self.scroll(ScrollDirection::Left);
      },
      MouseEventKind::ScrollRight => {
        self.scroll(ScrollDirection::Right);
      },
      _ => {},
    };
    Ok(None)
  }

  fn handle_key_events(&mut self, key: KeyEvent, app_state: &AppState) -> Result<Option<Action>> {
    if app_state.focus != Focus::Data {
      return Ok(None);
    }
    let input = Input::from(key);
    match input {
      Input { key: Key::Char('P'), .. } => {
        if let DataState::HasResults(rows) = &self.data_state {
          self.command_tx.clone().unwrap().send(Action::RequestExportData(rows.rows.len() as i64))?;
        }
      },
      Input { key: Key::Right, .. } | Input { key: Key::Char('l'), .. } => {
        self.scroll(ScrollDirection::Right);
      },
      Input { key: Key::Left, .. } | Input { key: Key::Char('h'), .. } => {
        self.scroll(ScrollDirection::Left);
      },
      Input { key: Key::Down, .. } | Input { key: Key::Char('j'), .. } => {
        self.scroll(ScrollDirection::Down);
      },
      Input { key: Key::Up, .. } | Input { key: Key::Char('k'), .. } => {
        self.scroll(ScrollDirection::Up);
      },
      Input { key: Key::Char('e'), .. } | Input { key: Key::Char('w'), .. } => {
        self.scrollable.next_column();
      },
      Input { key: Key::Char('b'), ctrl: false, .. } => {
        self.scrollable.prev_column();
      },
      Input { key: Key::Char('g'), .. } => {
        self.top();
      },
      Input { key: Key::Char('G'), .. } => {
        self.bottom();
      },
      Input { key: Key::Char('0'), .. } => {
        self.left();
      },
      Input { key: Key::Char('$'), .. } => {
        self.right();
      },
      Input { key: Key::Char('{'), .. }
      | Input { key: Key::Char('b'), ctrl: true, .. }
      | Input { key: Key::PageUp, .. } => {
        self.scrollable.pg_up();
      },
      Input { key: Key::Char('}'), .. }
      | Input { key: Key::Char('f'), ctrl: true, .. }
      | Input { key: Key::PageDown, .. } => {
        self.scrollable.pg_down();
      },
      Input { key: Key::Char('v'), .. } => {
        self.scrollable.transition_selection_mode(Some(SelectionMode::Cell));
      },
      Input { key: Key::Char('V'), .. } => {
        self.scrollable.transition_selection_mode(Some(SelectionMode::Row));
      },
      Input { key: Key::Enter, .. } => {
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
      Input { key: Key::Backspace, .. } => {
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
      Input { key: Key::Char('y'), .. } => {
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
        } else if let DataState::Explain(text) = &self.data_state {
          self.command_tx.clone().unwrap().send(Action::CopyData(text.to_string()))?;
          self.scrollable.transition_selection_mode(Some(SelectionMode::Copied));
        } else if let DataState::Error(err) = &self.data_state {
          self.command_tx.clone().unwrap().send(Action::CopyData(err.to_string()))?;
          self.scrollable.transition_selection_mode(Some(SelectionMode::Copied));
        }
      },
      Input { key: Key::Char('Y'), .. } => {
        if let DataState::HasResults(rows) = &self.data_state {
          self.command_tx.clone().unwrap().send(Action::RequestYankData(rows.rows.len() as i64))?;
        }
      },
      Input { key: Key::Esc, .. } => {
        self.scrollable.transition_selection_mode(None);
      },
      _ => {},
    };
    Ok(None)
  }

  fn update(&mut self, action: Action, app_state: &AppState) -> Result<Option<Action>> {
    if let Action::Query(query, confirmed, bypass) = action {
      self.scrollable.reset_scroll();
    } else if let Action::ExportData(format) = action {
      let DataState::HasResults(rows) = &self.data_state else {
        return Ok(None);
      };
      let name = format!("rainfrog_export_{}_rows_{}.csv", rows.rows.len(), chrono::Utc::now().timestamp());
      let mut writer = Writer::from_path(get_export_dir().join(name))?;
      writer.write_record(header_to_vec(&rows.headers))?;
      for row in &rows.rows {
        writer.write_record(row)?;
      }
      writer.flush()?;
      self.command_tx.clone().unwrap().send(Action::ExportDataFinished)?;
    } else if let Action::YankData = action {
      let DataState::HasResults(rows) = &self.data_state else {
        return Ok(None);
      };
      yank_to_clipboard(rows, app_state)?;
      self.command_tx.clone().unwrap().send(Action::YankDataFinished)?;
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

    let inner_area = block.inner(area);

    self.explain_max_x_offset = self.explain_width.saturating_sub(inner_area.width);
    self.explain_max_y_offset = self.explain_height.saturating_sub(inner_area.height);
    if let Some(ExplainOffsets { y_offset, x_offset }) = self.explain_scroll {
      self.explain_scroll = Some(ExplainOffsets {
        y_offset: y_offset.min(self.explain_max_y_offset),
        x_offset: x_offset.min(self.explain_max_x_offset),
      });
    }

    if let DataState::HasResults(Rows { rows, .. }) = &self.data_state {
      let (x, y) = self.scrollable.get_cell_offsets();
      let row = &rows[y];
      let title_string = match self.scrollable.get_selection_mode() {
        Some(SelectionMode::Row) => {
          format!(" 󰆼 results <alt+3> (row {} of {})", y.saturating_add(1), rows.len())
        },
        Some(SelectionMode::Cell) => {
          format!(" 󰆼 results <alt+3> (row {} of {}) - {} ", y.saturating_add(1), rows.len(), row[x as usize].clone())
        },
        Some(SelectionMode::Copied) => {
          format!(" 󰆼 results <alt+3> ({} rows) - copied! ", rows.len())
        },
        _ => format!(" 󰆼 results <alt+3> ({} rows)", rows.len()),
      };
      block = block.title(title_string);
    } else {
      let title_string = match self.scrollable.get_selection_mode() {
        Some(SelectionMode::Copied) => " 󰆼 results <alt+3> - copied! ",
        _ => " 󰆼 results <alt+3>",
      };
      block = block.title(title_string);
    }

    match &self.data_state {
      DataState::NoResults => {
        f.render_widget(Paragraph::new("no results").wrap(Wrap { trim: false }).block(block), area);
      },
      DataState::StatementCompleted(statement) => {
        f.render_widget(
          Paragraph::new(format!("{} statement completed", statement_type_string(Some(statement.clone()))))
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
        let mut paragraph = Paragraph::new(text.clone()).block(block);
        if let Some(offsets) = self.explain_scroll.clone() {
          paragraph = paragraph.scroll((offsets.y_offset, offsets.x_offset));
          f.render_widget(paragraph, area);
          let vertical_scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight).symbols(scrollbar::VERTICAL);
          let mut vertical_scrollbar_state =
            ScrollbarState::new(self.explain_max_y_offset as usize).position(offsets.y_offset as usize);
          let horizontal_scrollbar =
            Scrollbar::new(ScrollbarOrientation::HorizontalBottom).symbols(scrollbar::HORIZONTAL).thumb_symbol("▀");
          let mut horizontal_scrollbar_state =
            ScrollbarState::new(self.explain_max_x_offset as usize).position(offsets.x_offset as usize);
          match (self.explain_max_x_offset, self.explain_max_y_offset) {
            (0, 0) => {},
            (0, y) => {
              f.render_stateful_widget(
                vertical_scrollbar,
                area.inner(Margin { vertical: 1, horizontal: 0 }),
                &mut vertical_scrollbar_state,
              );
            },
            (x, 0) => {
              f.render_stateful_widget(
                horizontal_scrollbar,
                area.inner(Margin { vertical: 0, horizontal: 1 }),
                &mut horizontal_scrollbar_state,
              );
            },
            (x, y) => {
              f.render_stateful_widget(
                vertical_scrollbar,
                area.inner(Margin { vertical: 1, horizontal: 0 }),
                &mut vertical_scrollbar_state,
              );
              f.render_stateful_widget(
                horizontal_scrollbar,
                area.inner(Margin { vertical: 0, horizontal: 1 }),
                &mut horizontal_scrollbar_state,
              );
            },
          };
        }
      },
      DataState::HasResults(_) => {
        self.scrollable.block(block);
        self.scrollable.draw(f, area, app_state)?;
      },
      DataState::Error(e) => {
        f.render_widget(
          Paragraph::new(e.to_string()).style(Style::default().fg(Color::Red)).wrap(Wrap { trim: true }).block(block),
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

const SYSTEM_WINDOW_PROTOCOL: &str = env!("XDG_SESSION_TYPE");

fn yank_to_clipboard(rows: &Rows, app_state: &AppState) -> Result<()> {
  let (mut pipe, _) = clipboard(SYSTEM_WINDOW_PROTOCOL)?;
  let data_for_yank = DataForYank::new(rows, app_state).yank();
  pipe.write_all(data_for_yank.as_bytes())?;
  Ok(())
}

fn clipboard(system_window_protocol: &str) -> Result<(std::process::ChildStdin, std::process::Child)> {
  let mut child = match system_window_protocol {
    "wayland" => Command::new("wl-copy").stdin(Stdio::piped()).spawn(),
    "x11" => Command::new("xclip -selection clipboard").stdin(Stdio::piped()).spawn(),
    _ => Err(io::Error::new(io::ErrorKind::Unsupported, format!("Unsupported for {system_window_protocol}"))),
  }?;
  let pipe = child.stdin.take().expect("expected obtain clipboard's pipe");
  Ok((pipe, child))
}

struct DataForYank {
  sql: Vec<String>,
  table: Vec<VecDeque<String>>,
}

impl DataForYank {
  fn new(rows: &Rows, app_state: &AppState) -> Self {
    let sql = app_state.history.first().expect("expected the last SQL query in history").query_lines.clone();

    let headers: &Vec<String> = &rows.headers.iter().map(|h| h.name.clone()).collect();
    let rows = &rows.rows;

    let table = Self::to_columns(headers, rows);

    Self { sql, table }
  }

  fn yank(&mut self) -> String {
    self.table.iter_mut().enumerate().for_each(|(index, col)| Self::format_column(col, index));

    let mut buff = String::new();

    for statement in &self.sql {
      buff.push_str(statement);
      buff.push('\n');
    }

    buff.push('\n');

    while let Some(col) = self.table.first() {
      if col.is_empty() {
        break;
      }

      for col in &mut self.table {
        if let Some(cell) = col.pop_front() {
          buff.push_str(&cell);
        }
      }
      buff.push('\n');
    }

    buff
  }

  fn format_column(col: &mut VecDeque<String>, index: usize) {
    let width = col.iter().map(|s| s.len()).max().unwrap_or(1) + 1;

    let format_cell = |s: &str| {
      let prefix = if index == 0 { " " } else { "| " };
      let padding = " ".repeat(width.saturating_sub(s.len()));
      format!("{prefix}{s}{padding}")
    };

    col.iter_mut().for_each(|s| *s = format_cell(s));

    if let Some(header) = col.pop_front() {
      let div = if index == 0 { "-".repeat(width + 1) } else { format!("+{}", "-".repeat(width + 1)) };
      col.push_front(div);
      col.push_front(header);
    }
  }

  fn to_columns(headers: &[String], rows: &[Vec<String>]) -> Vec<VecDeque<String>> {
    headers
      .iter()
      .enumerate()
      .map(|(i, h)| {
        let mut col: VecDeque<String> = VecDeque::from([h.clone()]);
        rows.iter().filter_map(|row| row.get(i)).cloned().for_each(|v| col.push_back(v));
        col
      })
      .collect()
  }
}

#[cfg(test)]
mod yank {

  use crate::components::data::{DataForYank, clipboard};
  use std::collections::VecDeque;

  #[test]
  #[should_panic(expected = "Unsupported for whatever")]
  fn clipboard_error_unsupported() {
    const SYSTEM_WINDOW_PROTOCOL: &str = "whatever";
    let result = clipboard(SYSTEM_WINDOW_PROTOCOL).unwrap();
  }

  #[test]
  fn to_columns_is_works() {
    let headers = vec!["id".to_string(), "name".to_string(), "age".to_string()];
    let rows = vec![
      vec!["id1".to_string(), "name1".to_string(), "age1".to_string()],
      vec!["id2".to_string(), "name2".to_string(), "age2".to_string()],
      vec!["id3".to_string(), "name3".to_string(), "age3".to_string()],
    ];

    let result = DataForYank::to_columns(&headers, &rows);

    let expected = vec![
      VecDeque::from(["id".to_string(), "id1".to_string(), "id2".to_string(), "id3".to_string()]),
      VecDeque::from(["name".to_string(), "name1".to_string(), "name2".to_string(), "name3".to_string()]),
      VecDeque::from(["age".to_string(), "age1".to_string(), "age2".to_string(), "age3".to_string()]),
    ];

    assert_eq!(expected, result)
  }

  #[test]
  fn yank_is_works() {
    let headers = vec!["id".to_string(), "name".to_string(), "age".to_string()];
    let rows = vec![
      vec!["id1".to_string(), "name1".to_string(), "age1".to_string()],
      vec!["id2".to_string(), "name2".to_string(), "age2".to_string()],
      vec!["id3".to_string(), "name3".to_string(), "age3".to_string()],
    ];

    let mut data_to_yank = DataForYank {
      sql: vec!["select".to_string(), "*".to_string(), "from".to_string(), "something".to_string()],
      table: DataForYank::to_columns(&headers, &rows),
    };

    let result = data_to_yank.yank();

    let expected = "\
select
*
from
something

 id  | name  | age  
-----+-------+------
 id1 | name1 | age1 
 id2 | name2 | age2 
 id3 | name3 | age3 
";

    assert_eq!(expected, result)
  }

  #[test]
  fn fomart_column_is_work() {
    let table = vec![VecDeque::from([
      "states".to_string(),
      "Sao Paulo".to_string(),
      "Minas Gerais".to_string(),
      "Amazonas".to_string(),
      "Rio Grande do Sul".to_string(),
      "Mato Grosso".to_string(),
    ])];

    let mut data_to_yank = DataForYank { sql: vec!["".to_string()], table };

    data_to_yank.table.iter_mut().enumerate().for_each(|(index, col)| DataForYank::format_column(col, index));

    let result = data_to_yank.table.first().unwrap();

    let expected = VecDeque::from([
      format!(" states{}", " ".repeat(12)),
      "-".to_string().repeat(19),
      format!(" Sao Paulo{}", " ".repeat(9)),
      format!(" Minas Gerais{}", " ".repeat(6)),
      format!(" Amazonas{}", " ".repeat(10)),
      " Rio Grande do Sul ".to_string(),
      format!(" Mato Grosso{}", " ".repeat(7)),
    ]);

    assert_eq!(result, &expected)
  }
}
