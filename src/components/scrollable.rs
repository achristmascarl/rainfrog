use color_eyre::eyre::Result;
use ratatui::{
  buffer::Cell,
  prelude::*,
  widgets::{Block, WidgetRef},
};

use super::Component;

pub enum ScrollXDirection {
  Left,
  Right,
}

pub enum ScrollYDirection {
  Up,
  Down,
}

#[derive(Debug, Clone, Default)]
pub struct Scrollable<'a> {
  child_buffer: Buffer,
  parent_area: Rect,
  block: Option<Block<'a>>,
  x_offset: u16,
  y_offset: u16,
  max_offsets: MaxOffsets,
}

impl<'a> Scrollable<'a> {
  pub fn new() -> Self {
    Self {
      child_buffer: Buffer::empty(Rect::new(0, 0, 0, 0)),
      parent_area: Rect::new(0, 0, 0, 0),
      block: None,
      x_offset: 0,
      y_offset: 0,
      max_offsets: MaxOffsets { max_x_offset: 0, max_y_offset: 0 },
    }
  }

  pub fn child(&mut self, child_widget: Box<dyn WidgetRef>, max_height: u16, max_width: u16) -> &mut Self {
    let mut buf = Buffer::empty(Rect::new(0, 0, max_width, max_height));
    child_widget.render_ref(buf.area, &mut buf);
    let child_buffer = clamp(buf);
    self.child_buffer = child_buffer;
    self
  }

  pub fn block(&mut self, block: Block<'a>) -> &mut Self {
    self.block = Some(block);
    self
  }

  pub fn scroll_x(&mut self, direction: ScrollXDirection) -> &mut Self {
    match direction {
      ScrollXDirection::Left => {
        if self.x_offset > 0 {
          self.x_offset -= 1;
        }
      },
      ScrollXDirection::Right => {
        if self.x_offset < self.max_offsets.max_x_offset {
          self.x_offset += 1;
        }
      },
    }
    self
  }

  pub fn scroll_y(&mut self, direction: ScrollYDirection) -> &mut Self {
    match direction {
      ScrollYDirection::Up => {
        if self.y_offset > 0 {
          self.y_offset -= 1;
        }
      },
      ScrollYDirection::Down => {
        if self.y_offset < self.max_offsets.max_y_offset {
          self.y_offset += 1;
        }
      },
    }
    self
  }

  pub fn reset_scroll(&mut self) -> &mut Self {
    self.x_offset = 0;
    self.y_offset = 0;
    self
  }

  fn widget(&'a self) -> impl Widget + 'a {
    Renderer::new(self)
  }

  pub fn log(&self) {
    let buf_height = self.child_buffer.area.height;
    let buf_width = self.child_buffer.area.width;
    for n in 0..buf_height {
      let mut line: String = String::from("");
      let cells =
        self.child_buffer.content.to_vec()[((n * buf_width) as usize)..(((n + 1) * buf_width) as usize)].to_vec();
      for cell in cells.iter() {
        line += cell.symbol();
      }
      log::info!("{}", line.as_str());
    }
  }

  pub fn debug_log(&self) {
    let buf_height = self.child_buffer.area.height;
    let buf_width = self.child_buffer.area.width;
    for n in 0..buf_height {
      let mut line: String = String::from("");
      let cells =
        self.child_buffer.content.to_vec()[((n * buf_width) as usize)..(((n + 1) * buf_width) as usize)].to_vec();
      for cell in cells.iter() {
        line += cell.symbol();
      }
      log::info!(
        "rendering line {}/{}, length {}, last symbol {}, last symbol is blank {}:",
        n,
        buf_height,
        line.len(),
        cells[cells.len() - 1].symbol(),
        cells[cells.len() - 1].symbol() == " "
      );

      log::info!("{}", line.as_str());
    }
  }
}

impl<'a> Component for Scrollable<'a> {
  fn draw(&mut self, f: &mut Frame<'_>, area: Rect) -> Result<()> {
    self.parent_area = area;
    self.max_offsets = get_max_offsets(&self.child_buffer, &self.parent_area, &self.block);
    f.render_widget(self.widget(), area);
    Ok(())
  }
}

#[derive(Debug, Clone, Default)]
struct MaxOffsets {
  max_x_offset: u16,
  max_y_offset: u16,
}

fn get_max_offsets(child_buffer: &Buffer, parent_area: &Rect, parent_block: &Option<Block>) -> MaxOffsets {
  parent_block.render_ref(*parent_area, &mut child_buffer.clone());
  let render_area = parent_block.inner_if_some(*parent_area);
  if render_area.is_empty() {
    return MaxOffsets { max_x_offset: 0, max_y_offset: 0 };
  }
  let parent_width = render_area.width as i32;
  let parent_height = render_area.height as i32;
  let content_height = child_buffer.area.height as i32;
  let content_width = child_buffer.area.width as i32;
  MaxOffsets {
    max_x_offset: Ord::max(content_width - parent_width, 0) as u16,
    max_y_offset: Ord::max(content_height - parent_height, 0) as u16,
  }
}

fn clamp(buf: Buffer) -> Buffer {
  let height = buf.area.height;
  let width = buf.area.width;
  let mut used_height: u16 = 0;
  let mut used_width: u16 = 0;
  for y in (0..height).rev() {
    let row = get_row(&buf.content, y, width);
    for x in (0..width).rev() {
      let cell = &row[x as usize];
      if cell.symbol() != " " {
        used_height = std::cmp::max(used_height, y + 1);
        used_width = std::cmp::max(used_width, x + 1);
      }
    }
  }
  let mut content: Vec<ratatui::buffer::Cell> = Vec::new();
  for y in 0..used_height {
    let row = get_row(&buf.content, y, width);
    for x in 0..used_width {
      content.push(row[x as usize].to_owned());
    }
  }
  Buffer { area: Rect::new(0, 0, used_width, used_height), content }
}

// based on scrolling approach from tui-textarea:
// https://github.com/rhysd/tui-textarea/blob/main/src/widget.rs
pub struct Renderer<'a>(&'a Scrollable<'a>);

impl<'a> Renderer<'a> {
  pub fn new(scrollable: &'a Scrollable<'a>) -> Self {
    Self(scrollable)
  }
}

impl<'a> Widget for Renderer<'a> {
  fn render(self, area: Rect, buf: &mut Buffer) {
    let scrollable = self.0;
    scrollable.block.render_ref(area, buf);
    let render_area = scrollable.block.inner_if_some(area);
    if render_area.is_empty() {
      return;
    }
    let area = render_area.intersection(buf.area);
    let content_height = scrollable.child_buffer.area.height;
    let content_width = scrollable.child_buffer.area.width;
    let max_x = Ord::min(area.x.saturating_add(area.width), area.x.saturating_add(content_width));
    let max_y = Ord::min(area.y.saturating_add(area.height), area.y.saturating_add(content_height));
    for y in area.y..max_y {
      let content_y = y + scrollable.y_offset - area.y;
      let row = get_row(&scrollable.child_buffer.content, content_y, content_width);
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
