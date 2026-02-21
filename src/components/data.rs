use std::collections::VecDeque;

use color_eyre::eyre::{self, Result};
use crossterm::event::{KeyEvent, MouseEventKind};
use csv::Writer;
use ratatui::{prelude::*, symbols::scrollbar, widgets::*};
use sqlparser::ast::Statement;
use tokio::sync::mpsc::UnboundedSender;
use tui_textarea::{Input, Key};

use super::{
  Frame,
  scroll_table::{COLUMN_SPACING, ScrollDirection, ScrollTable, SelectionMode},
};
use crate::{
  action::Action,
  app::AppState,
  components::Component,
  config::Config,
  database::{Rows, header_to_vec, statement_type_string},
  focus::Focus,
  utils::get_export_dir,
};

const MAX_COLUMN_WIDTH: u16 = 36;
const TITLE_CELL_PREVIEW_MAX_CHARS: usize = 96;

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
            self.explain_scroll = Some(ExplainOffsets {
              y_offset: offsets.y_offset.saturating_sub(1),
              x_offset: offsets.x_offset,
            });
          },
          ScrollDirection::Down => {
            self.explain_scroll = Some(ExplainOffsets {
              y_offset: offsets.y_offset.saturating_add(1),
              x_offset: offsets.x_offset,
            });
          },
          ScrollDirection::Left => {
            self.explain_scroll = Some(ExplainOffsets {
              y_offset: offsets.y_offset,
              x_offset: offsets.x_offset.saturating_sub(2),
            });
          },
          ScrollDirection::Right => {
            self.explain_scroll = Some(ExplainOffsets {
              y_offset: offsets.y_offset,
              x_offset: offsets.x_offset.saturating_add(2),
            });
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
          self.explain_scroll =
            Some(ExplainOffsets { y_offset: self.explain_max_y_offset, x_offset });
        },
        _ => {
          self.explain_scroll =
            Some(ExplainOffsets { y_offset: self.explain_max_y_offset, x_offset: 0 });
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
          self.explain_scroll =
            Some(ExplainOffsets { y_offset, x_offset: self.explain_max_x_offset });
        },
        _ => {
          self.explain_scroll =
            Some(ExplainOffsets { y_offset: 0, x_offset: self.explain_max_x_offset });
        },
      }
    } else if let DataState::HasResults(_) = self.data_state {
      self.scrollable.last_column();
    }
  }

  fn column_widths(&self, rows: &Rows) -> Vec<u16> {
    if self.config.settings.data_compact_columns.unwrap_or(false) {
      Self::compact_column_widths(rows)
    } else {
      vec![MAX_COLUMN_WIDTH; rows.headers.len()]
    }
  }

  fn compact_column_widths(rows: &Rows) -> Vec<u16> {
    let column_count = rows.headers.len();
    if column_count == 0 {
      return Vec::new();
    }
    let mut widths = vec![0_usize; column_count];
    for (index, header) in rows.headers.iter().enumerate() {
      widths[index] =
        Self::cell_display_width(&header.name).max(Self::cell_display_width(&header.type_name));
    }
    for row in &rows.rows {
      for (index, value) in row.iter().enumerate().take(column_count) {
        widths[index] = widths[index].max(Self::cell_display_width(value));
      }
    }
    widths
      .into_iter()
      .map(|len| {
        let len_with_padding = len.saturating_add(1);
        let clamped = std::cmp::min(len_with_padding, MAX_COLUMN_WIDTH as usize);
        std::cmp::max(1, clamped) as u16
      })
      .collect()
  }

  fn cell_display_width(value: &str) -> usize {
    value.chars().take(MAX_COLUMN_WIDTH as usize).count()
  }

  fn clamp_render_text(value: &str, max_chars: usize) -> String {
    if max_chars == 0 || value.is_empty() {
      return String::new();
    }
    let mut chars_seen = 0_usize;
    for (idx, _) in value.char_indices() {
      if chars_seen == max_chars {
        return value[..idx].to_owned();
      }
      chars_seen = chars_seen.saturating_add(1);
    }
    value.to_owned()
  }

  fn preview_text(value: &str, max_chars: usize) -> String {
    if max_chars == 0 || value.is_empty() {
      return String::new();
    }
    let mut chars_seen = 0_usize;
    for (idx, _) in value.char_indices() {
      if chars_seen == max_chars {
        let mut preview = value[..idx].to_owned();
        preview.push_str("...");
        return preview;
      }
      chars_seen = chars_seen.saturating_add(1);
    }
    value.to_owned()
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
          self.explain_width =
            rows.rows.iter().fold(0_u16, |acc, r| acc.max(r.join(" ").len() as u16));
          self.explain_height = rows.rows.len() as u16;
          self.explain_scroll = Some(ExplainOffsets { y_offset: 0, x_offset: 0 });
          self.data_state =
            DataState::Explain(Text::from_iter(rows.rows.iter().map(|r| r.join(" "))));
        } else {
          let row_spacing_enabled = self.config.settings.data_row_spacer.unwrap_or(false);
          let row_bottom_margin: u16 = if row_spacing_enabled { 1 } else { 0 };
          let header_height: u16 = 2;
          let data_row_offset = header_height.saturating_add(row_bottom_margin);
          let column_widths = self.column_widths(&rows);
          let header_row = Row::new(
            rows
              .headers
              .iter()
              .enumerate()
              .map(|(index, h)| {
                let col_width =
                  column_widths.get(index).copied().unwrap_or(MAX_COLUMN_WIDTH) as usize;
                let header_name = Self::clamp_render_text(&h.name, col_width);
                let header_type = Self::clamp_render_text(&h.type_name, col_width);
                Cell::from(format!("{header_name}\n{header_type}"))
              })
              .collect::<Vec<Cell>>(),
          )
          .height(header_height)
          .bottom_margin(row_bottom_margin);
          let value_rows = rows.rows.iter().map(|r| {
            Row::new(
              r.iter()
                .enumerate()
                .map(|(index, value)| {
                  let col_width =
                    column_widths.get(index).copied().unwrap_or(MAX_COLUMN_WIDTH) as usize;
                  Self::clamp_render_text(value, col_width)
                })
                .collect::<Vec<String>>(),
            )
            .bottom_margin(row_bottom_margin)
          });
          let buf_table = Table::new(value_rows, column_widths.clone())
            .header(header_row)
            .style(Style::default())
            .column_spacing(COLUMN_SPACING)
            .row_highlight_style(Style::default().fg(Color::LightBlue).reversed().bold());
          self.scrollable.set_table(buf_table, column_widths, rows.rows.len(), data_row_offset);
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
          self
            .command_tx
            .clone()
            .unwrap()
            .send(Action::RequestExportData(rows.rows.len() as i64))?;
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
          let should_render_as_paragraph =
            rows.len() == 1 && rows.first().is_some_and(|row| row.len() == 1);
          if should_render_as_paragraph {
            let cell = rows.first().and_then(|row| row.first()).cloned().unwrap_or_default();
            self.command_tx.clone().unwrap().send(Action::CopyData(cell))?;
            self.scrollable.transition_selection_mode(Some(SelectionMode::Copied));
          } else {
            let (x, y) = self.scrollable.get_cell_offsets();
            let row = &rows[y];
            match self.scrollable.get_selection_mode() {
              Some(SelectionMode::Row) => {
                let row_string = row.join(", ");
                self.command_tx.clone().unwrap().send(Action::CopyData(row_string))?;
                self.scrollable.transition_selection_mode(Some(SelectionMode::Copied));
              },
              Some(SelectionMode::Cell) => {
                if let Some(cell) = row.get(x) {
                  self.command_tx.clone().unwrap().send(Action::CopyData(cell.clone()))?;
                  self.scrollable.transition_selection_mode(Some(SelectionMode::Copied));
                }
              },
              _ => {},
            }
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
          self.command_tx.clone().unwrap().send(Action::RequestYankAll(rows.rows.len() as i64))?;
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
        self.command_tx.clone().unwrap().send(Action::ExportDataFinished)?;
        return Ok(None);
      };
      let name =
        format!("rainfrog_export_{}_rows_{}.csv", rows.rows.len(), chrono::Utc::now().timestamp());
      let mut writer = Writer::from_path(get_export_dir().join(name))?;
      writer.write_record(header_to_vec(&rows.headers))?;
      for row in &rows.rows {
        writer.write_record(row)?;
      }
      writer.flush()?;
      self.command_tx.clone().unwrap().send(Action::ExportDataFinished)?;
    } else if let Action::YankAll = action {
      let DataState::HasResults(rows) = &self.data_state else {
        return Ok(None);
      };
      let table_for_yank = TableForYank::new(rows, app_state).yank();
      self.command_tx.clone().unwrap().send(Action::CopyData(table_for_yank))?;
      self.scrollable.transition_selection_mode(Some(SelectionMode::Copied));
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
          let cell = row
            .get(x)
            .map(|c| Self::preview_text(c, TITLE_CELL_PREVIEW_MAX_CHARS))
            .unwrap_or_default();
          format!(" 󰆼 results <alt+3> (row {} of {}) - {} ", y.saturating_add(1), rows.len(), cell)
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
          Paragraph::new(format!(
            "{} statement completed",
            statement_type_string(Some(statement.clone()))
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
      DataState::Explain(text) => {
        let mut paragraph = Paragraph::new(text.clone()).block(block);
        if let Some(offsets) = self.explain_scroll.clone() {
          paragraph = paragraph.scroll((offsets.y_offset, offsets.x_offset));
          f.render_widget(paragraph, area);
          let vertical_scrollbar =
            Scrollbar::new(ScrollbarOrientation::VerticalRight).symbols(scrollbar::VERTICAL);
          let mut vertical_scrollbar_state =
            ScrollbarState::new(self.explain_max_y_offset as usize)
              .position(offsets.y_offset as usize);
          let horizontal_scrollbar = Scrollbar::new(ScrollbarOrientation::HorizontalBottom)
            .symbols(scrollbar::HORIZONTAL)
            .thumb_symbol("▀");
          let mut horizontal_scrollbar_state =
            ScrollbarState::new(self.explain_max_x_offset as usize)
              .position(offsets.x_offset as usize);
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
      DataState::HasResults(rows) => {
        let should_render_as_paragraph = rows.headers.len() == 1
          && rows.rows.len() == 1
          && rows.rows.first().is_some_and(|row| row.len() == 1);
        if should_render_as_paragraph {
          let header = rows.headers.first();
          let column_name = header.map(|h| h.name.clone()).unwrap_or_else(|| "column".to_owned());
          let column_type =
            header.map(|h| h.type_name.clone()).unwrap_or_else(|| "unknown".to_owned());
          let value = rows.rows.first().and_then(|row| row.first()).cloned().unwrap_or_default();
          let paragraph_text = format!("{column_name}\n{column_type}\n\n{value}");
          f.render_widget(
            Paragraph::new(paragraph_text).wrap(Wrap { trim: false }).block(block),
            area,
          );
        } else {
          self.scrollable.block(block);
          self.scrollable.draw(f, area, app_state)?;
        }
      },
      DataState::Error(e) => {
        f.render_widget(
          Paragraph::new(e.to_string())
            .style(Style::default().fg(Color::Red))
            .wrap(Wrap { trim: true })
            .block(block),
          area,
        );
      },
      DataState::Loading => {
        f.render_widget(
          Paragraph::new(Text::from("loading...").fg(Color::Green))
            .wrap(Wrap { trim: false })
            .block(block),
          area,
        );
      },
      DataState::Cancelled => {
        f.render_widget(
          Paragraph::new(Text::from("query cancelled.").fg(Color::Yellow))
            .wrap(Wrap { trim: false })
            .block(block),
          area,
        );
      },
    }

    Ok(())
  }
}

struct TableForYank {
  sql: Vec<String>,
  table: Vec<VecDeque<String>>,
}

impl TableForYank {
  fn new(rows: &Rows, app_state: &AppState) -> Self {
    let sql = app_state
      .history
      .first()
      .expect("expected the last SQL query in history")
      .query_lines
      .clone();

    let headers: &Vec<String> = &rows.headers.iter().map(|h| h.name.clone()).collect();
    let rows = &rows.rows;

    let table = Self::to_columns(headers, rows);

    Self { sql, table }
  }

  fn yank(&mut self) -> String {
    let last_index = self.table.len() - 1;
    self
      .table
      .iter_mut()
      .enumerate()
      .for_each(|(index, col)| Self::format_column(col, index, last_index));

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

  fn format_column(col: &mut VecDeque<String>, index: usize, last_index: usize) {
    let width = col.iter().map(|s| s.len()).max().unwrap_or(1) + 1;

    let format_cell = |s: &str| {
      let prefix = if index == 0 { " " } else { "| " };
      let padding =
        if index == last_index { " ".repeat(0) } else { " ".repeat(width.saturating_sub(s.len())) };
      format!("{prefix}{s}{padding}")
    };

    col.iter_mut().for_each(|s| *s = format_cell(s));

    if let Some(header) = col.pop_front() {
      let div =
        if index == 0 { "-".repeat(width + 1) } else { format!("+{}", "-".repeat(width + 1)) };
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

  use std::collections::VecDeque;

  use crate::components::data::TableForYank;

  #[test]
  fn to_columns_is_works() {
    let headers = vec!["id".to_string(), "name".to_string(), "age".to_string()];
    let rows = vec![
      vec!["id1".to_string(), "name1".to_string(), "age1".to_string()],
      vec!["id2".to_string(), "name2".to_string(), "age2".to_string()],
      vec!["id3".to_string(), "name3".to_string(), "age3".to_string()],
    ];

    let result = TableForYank::to_columns(&headers, &rows);

    let expected = vec![
      VecDeque::from(["id".to_string(), "id1".to_string(), "id2".to_string(), "id3".to_string()]),
      VecDeque::from([
        "name".to_string(),
        "name1".to_string(),
        "name2".to_string(),
        "name3".to_string(),
      ]),
      VecDeque::from([
        "age".to_string(),
        "age1".to_string(),
        "age2".to_string(),
        "age3".to_string(),
      ]),
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

    let mut data_to_yank = TableForYank {
      sql: vec!["select".to_string(), "*".to_string(), "from".to_string(), "something".to_string()],
      table: TableForYank::to_columns(&headers, &rows),
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
  }
}
