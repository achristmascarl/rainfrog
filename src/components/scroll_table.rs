use color_eyre::eyre::Result;
use ratatui::{
  buffer::Cell,
  prelude::*,
  widgets::{Block, Scrollbar, ScrollbarOrientation, ScrollbarState, Table, TableState, WidgetRef},
};
use symbols::scrollbar;

use super::Component;
use crate::app::AppState;

pub const COLUMN_SPACING: u16 = 1;

pub enum ScrollDirection {
  Left,
  Right,
  Up,
  Down,
}

#[derive(Debug, Clone)]
pub enum SelectionMode {
  Row,
  Cell,
  Copied,
}

#[derive(Debug, Clone, Default)]
pub struct ScrollTable<'a> {
  table: Table<'a>,
  parent_area: Rect,
  block: Option<Block<'a>>,
  pg_height: u16,
  requested_width: u16,
  column_widths: Vec<u16>,
  column_offsets: Vec<u16>,
  max_height: u16,
  x_offset: u16,
  y_offset: usize,
  max_x_offset: u16,
  max_y_offset: usize,
  selection_mode: Option<SelectionMode>,
}

impl<'a> ScrollTable<'a> {
  pub fn new() -> Self {
    Self {
      table: Table::default(),
      parent_area: Rect::new(0, 0, 0, 0),
      block: None,
      pg_height: 0,
      requested_width: 0,
      column_widths: Vec::new(),
      column_offsets: Vec::new(),
      max_height: 0,
      x_offset: 0,
      y_offset: 0,
      max_x_offset: 0,
      max_y_offset: 0,
      selection_mode: None,
    }
  }

  pub fn set_table(&mut self, table: Table<'a>, column_widths: Vec<u16>, row_count: usize) -> &mut Self {
    let requested_width = Self::requested_width(&column_widths);
    let max_height = u16::MAX.saturating_div(std::cmp::max(1, requested_width));
    self.table = table;
    self.column_widths = column_widths;
    self.column_offsets = Self::build_offsets(&self.column_widths);
    self.requested_width = requested_width;
    self.max_height = max_height;
    self.max_y_offset = row_count.saturating_sub(1);
    self
  }

  pub fn block(&mut self, block: Block<'a>) -> &mut Self {
    self.block = Some(block);
    self
  }

  pub fn scroll(&mut self, direction: ScrollDirection) -> &mut Self {
    match direction {
      ScrollDirection::Left => self.x_offset = self.x_offset.saturating_sub(2),
      ScrollDirection::Right => self.x_offset = std::cmp::min(self.x_offset.saturating_add(2), self.max_x_offset),
      ScrollDirection::Up => self.y_offset = self.y_offset.saturating_sub(1),
      ScrollDirection::Down => self.y_offset = std::cmp::min(self.y_offset.saturating_add(1), self.max_y_offset),
    }
    self
  }

  pub fn next_column(&mut self) -> &mut Self {
    if self.column_widths.is_empty() {
      return self;
    }
    let current_index = self.current_column_index().unwrap_or(0);
    if current_index + 1 < self.column_offsets.len() {
      self.x_offset = self.column_offsets[current_index + 1];
    } else {
      self.x_offset = self.max_x_offset;
    }
    self
  }

  pub fn prev_column(&mut self) -> &mut Self {
    if self.column_widths.is_empty() {
      return self;
    }
    let current_index = self.current_column_index().unwrap_or(0);
    let current_start = self.column_offsets[current_index];
    if self.x_offset > current_start {
      self.x_offset = current_start;
    } else if current_index > 0 {
      self.x_offset = self.column_offsets[current_index - 1];
    } else {
      self.x_offset = 0;
    }
    self
  }

  pub fn pg_up(&mut self) -> &mut Self {
    self.y_offset = self.y_offset.saturating_sub(std::cmp::max(
      1,
      self.pg_height.saturating_div(2).saturating_sub(
        u16::from(self.pg_height.is_multiple_of(2)), // always round down
      ) as usize,
    ));
    self
  }

  pub fn pg_down(&mut self) -> &mut Self {
    let new_y_offset = self.y_offset.saturating_add(std::cmp::max(
      1,
      self.pg_height.saturating_div(2).saturating_sub(
        u16::from(self.pg_height.is_multiple_of(2)), // always rounds down
      ) as usize,
    ));
    self.y_offset = std::cmp::min(self.max_y_offset, new_y_offset);
    self
  }

  pub fn bottom_row(&mut self) -> &mut Self {
    self.y_offset = self.max_y_offset;
    self
  }

  pub fn top_row(&mut self) -> &mut Self {
    self.y_offset = 0;
    self
  }

  pub fn last_column(&mut self) -> &mut Self {
    self.x_offset = self.max_x_offset;
    self
  }

  pub fn first_column(&mut self) -> &mut Self {
    self.x_offset = 0;
    self
  }

  pub fn reset_scroll(&mut self) -> &mut Self {
    self.x_offset = 0;
    self.y_offset = 0;
    self
  }

  pub fn get_cell_offsets(&self) -> (usize, usize) {
    let col_index = self.current_column_index().unwrap_or(0);
    (col_index, self.y_offset)
  }

  pub fn get_selection_mode(&self) -> Option<SelectionMode> {
    self.selection_mode.clone()
  }

  pub fn transition_selection_mode(&mut self, new_mode: Option<SelectionMode>) -> &mut Self {
    self.selection_mode = new_mode;
    self
  }

  fn get_max_x_offset(&self, parent_area: &Rect, parent_block: &Option<Block>) -> u16 {
    let render_area = parent_block.inner_if_some(*parent_area);
    if render_area.is_empty() {
      return 0_u16;
    }
    self.column_offsets.last().copied().unwrap_or(0)
  }

  fn requested_width(column_widths: &[u16]) -> u16 {
    if column_widths.is_empty() {
      return 0;
    }
    let width_total = column_widths.iter().fold(0_u16, |acc, width| acc.saturating_add(*width));
    let gaps = column_widths.len().saturating_sub(1);
    let gaps_u16 = std::cmp::min(gaps, u16::MAX as usize) as u16;
    let spacing_total = COLUMN_SPACING.saturating_mul(gaps_u16);
    width_total.saturating_add(spacing_total)
  }

  fn build_offsets(column_widths: &[u16]) -> Vec<u16> {
    let mut offsets = Vec::with_capacity(column_widths.len());
    let mut current = 0_u16;
    for (index, width) in column_widths.iter().enumerate() {
      offsets.push(current);
      current = current.saturating_add(*width);
      if index + 1 < column_widths.len() {
        current = current.saturating_add(COLUMN_SPACING);
      }
    }
    offsets
  }

  fn current_column_index(&self) -> Option<usize> {
    if self.column_offsets.is_empty() {
      return None;
    }
    let mut current_index = 0;
    for (index, start) in self.column_offsets.iter().enumerate() {
      if *start > self.x_offset {
        break;
      }
      current_index = index;
    }
    Some(current_index)
  }

  fn selected_column_bounds(&self) -> Option<(u16, u16)> {
    let index = self.current_column_index()?;
    let start = *self.column_offsets.get(index)?;
    let width = *self.column_widths.get(index)?;
    Some((start, start.saturating_add(width)))
  }

  fn is_within_selected_column(&self, position: u16) -> bool {
    if let Some((start, end)) = self.selected_column_bounds() {
      return position >= start && position < end;
    }
    false
  }

  fn widget(&'a self) -> Renderer<'a> {
    Renderer::new(self, self.y_offset)
  }
}

impl Component for ScrollTable<'_> {
  fn draw(&mut self, f: &mut Frame<'_>, area: Rect, app_state: &AppState) -> Result<()> {
    self.parent_area = area;
    let render_area = self.block.inner_if_some(area);
    self.pg_height = std::cmp::min(self.max_height, render_area.height).saturating_sub(3);
    self.max_x_offset = self.get_max_x_offset(&self.parent_area, &self.block);
    let max_x_offset = self.max_x_offset;
    let x_offset = self.x_offset;
    f.render_widget(self.widget(), area);
    let vertical_scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight).symbols(scrollbar::VERTICAL);
    let mut vertical_scrollbar_state = ScrollbarState::new(self.max_y_offset).position(self.y_offset);
    let horizontal_scrollbar =
      Scrollbar::new(ScrollbarOrientation::HorizontalBottom).symbols(scrollbar::HORIZONTAL).thumb_symbol("▀");
    let mut horizontal_scrollbar_state = ScrollbarState::new(max_x_offset as usize).position(x_offset as usize);
    match (self.max_x_offset, self.max_y_offset) {
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
    Ok(())
  }
}

// based on scrolling approach from tui-textarea:
// https://github.com/rhysd/tui-textarea/blob/main/src/widget.rs
pub struct Renderer<'a>(&'a ScrollTable<'a>, TableState);

impl<'a> Renderer<'a> {
  pub fn new(scrollable: &'a ScrollTable<'a>, y_offset: usize) -> Self {
    Self(scrollable, TableState::default().with_offset(y_offset))
  }

  pub fn offset(&self) -> usize {
    self.1.offset()
  }
}

impl Widget for Renderer<'_> {
  fn render(self, area: Rect, buf: &mut Buffer) {
    let scrollable = self.0;
    let table = &scrollable.table;
    let mut table_state = self.1;
    let current_offset = table_state.offset();
    if let Some(SelectionMode::Row) = scrollable.selection_mode {
      table_state = table_state.with_selected(current_offset);
    }
    scrollable.block.render_ref(area, buf);
    let render_area = scrollable.block.inner_if_some(area);
    if render_area.is_empty() {
      return;
    }
    let area = render_area.intersection(buf.area);
    let mut content_buf = Buffer::empty(Rect::new(
      0,
      0,
      scrollable.requested_width,
      std::cmp::min(scrollable.max_height, render_area.height),
    ));
    ratatui::widgets::StatefulWidgetRef::render_ref(table, content_buf.area, &mut content_buf, &mut table_state);
    let content_width = content_buf.area.width;
    let content_height = content_buf.area.height;
    let max_x = std::cmp::min(area.x.saturating_add(area.width), area.x.saturating_add(content_width));
    let max_y = std::cmp::min(area.y.saturating_add(area.height), area.y.saturating_add(content_height));
    for y in area.y..max_y {
      let content_y = y - area.y;
      let row = get_row(&content_buf.content, content_y, content_width);
      for x in area.x..max_x {
        let content_x = x + scrollable.x_offset - area.x;
        let default_cell = Cell::default();
        let cell = match &row.len().saturating_sub(1).saturating_sub(content_x as usize) {
          0 => &default_cell,
          _ => &row[content_x as usize],
        };
        let should_highlight = matches!(scrollable.selection_mode.as_ref(), Some(SelectionMode::Cell))
          && content_y == 3
          && scrollable.is_within_selected_column(content_x);
        let style = if should_highlight {
          Style::default().fg(Color::LightBlue).reversed().bold().italic()
        } else {
          cell.style()
        };
        buf
          .cell_mut(Position::from((x, y)))
          .unwrap()
          .set_symbol(cell.symbol())
          .set_fg(cell.fg)
          .set_bg(cell.bg)
          .set_skip(cell.skip)
          .set_style(style);
      }
    }
  }
}

fn get_row(content: &[Cell], row: u16, width: u16) -> Vec<Cell> {
  content[((row * width) as usize)..(((row + 1) * width) as usize)].to_vec()
}
