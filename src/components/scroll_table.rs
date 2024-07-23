use std::{borrow::BorrowMut, cell::RefCell};

use color_eyre::eyre::Result;
use ratatui::{
  buffer::Cell,
  prelude::*,
  widgets::{
    Block, ScrollDirection as RatatuiScrollDir, Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidgetRef,
    Table, TableState, WidgetRef,
  },
};
use symbols::scrollbar;

use super::Component;
use crate::app::AppState;

pub enum ScrollDirection {
  Left,
  Right,
  Up,
  Down,
}

#[derive(Debug, Clone, Default)]
pub struct ScrollTable<'a> {
  viewport_buffer: Buffer,
  table: Table<'a>,
  parent_area: Rect,
  block: Option<Block<'a>>,
  requested_width: u16,
  column_width: u16,
  max_height: u16,
  x_offset: u16,
  y_offset: usize,
  max_x_offset: u16,
  max_y_offset: usize,
}

impl<'a> ScrollTable<'a> {
  pub fn new() -> Self {
    Self {
      viewport_buffer: Buffer::empty(Rect::new(0, 0, 0, 0)),
      table: Table::default(),
      parent_area: Rect::new(0, 0, 0, 0),
      block: None,
      requested_width: 0,
      column_width: 0,
      max_height: 0,
      x_offset: 0,
      y_offset: 0,
      max_x_offset: 0,
      max_y_offset: 0,
    }
  }

  pub fn set_table(
    &mut self,
    table: Box<Table<'a>>,
    column_count: usize,
    row_count: usize,
    column_width: u16,
  ) -> &mut Self {
    let requested_width = column_width.saturating_mul(column_count as u16);
    let max_height = u16::MAX.saturating_div(requested_width);
    self.table = *table;
    self.column_width = column_width;
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
      ScrollDirection::Right => self.x_offset = Ord::min(self.x_offset.saturating_add(2), self.max_x_offset),
      ScrollDirection::Up => self.y_offset = self.y_offset.saturating_sub(1),
      ScrollDirection::Down => self.y_offset = Ord::min(self.y_offset.saturating_add(1), self.max_y_offset),
    }
    self
  }

  pub fn next_column(&mut self) -> &mut Self {
    let x_over = self.x_offset % self.column_width;
    self.x_offset = Ord::min(self.x_offset.saturating_add(self.column_width).saturating_sub(x_over), self.max_x_offset);
    self
  }

  pub fn prev_column(&mut self) -> &mut Self {
    let x_over = self.x_offset % self.column_width;
    match x_over {
      0 => {
        self.x_offset = self.x_offset.saturating_sub(self.column_width);
      },
      x => {
        self.x_offset = self.x_offset.saturating_sub(x);
      },
    }
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

  fn widget(&'a self) -> Renderer<'a> {
    Renderer::new(self, self.y_offset)
  }
}

impl<'a> Component for ScrollTable<'a> {
  fn draw(&mut self, f: &mut Frame<'_>, area: Rect, app_state: &AppState) -> Result<()> {
    self.parent_area = area;
    self.max_x_offset = get_max_x_offset(self.requested_width, &self.parent_area, &self.block);
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

fn get_max_x_offset(requested_width: u16, parent_area: &Rect, parent_block: &Option<Block>) -> u16 {
  let render_area = parent_block.inner_if_some(*parent_area);
  if render_area.is_empty() {
    return 0_u16;
  }
  let parent_width = render_area.width;
  requested_width.saturating_sub(parent_width)
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

impl<'a> Widget for Renderer<'a> {
  fn render(self, area: Rect, buf: &mut Buffer) {
    let scrollable = self.0;
    let table = &scrollable.table;
    let mut table_state = self.1;
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
    let max_x = Ord::min(area.x.saturating_add(area.width), area.x.saturating_add(content_width));
    let max_y = Ord::min(area.y.saturating_add(area.height), area.y.saturating_add(content_height));
    for y in area.y..max_y {
      let content_y = y - area.y;
      let row = get_row(&content_buf.content, content_y, content_width);
      for x in area.x..max_x {
        let content_x = x + scrollable.x_offset - area.x;
        let cell = &row[content_x as usize];
        buf.get_mut(x, y).set_symbol(cell.symbol()).set_fg(cell.fg).set_bg(cell.bg).set_skip(cell.skip);
      }
    }
  }
}

fn get_row(content: &[Cell], row: u16, width: u16) -> Vec<Cell> {
  content[((row * width) as usize)..(((row + 1) * width) as usize)].to_vec()
}
