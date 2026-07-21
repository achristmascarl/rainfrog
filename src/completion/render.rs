use ratatui::{
  Frame,
  layout::{Position, Rect},
  style::{Color, Modifier, Style},
  text::{Line, Span},
  widgets::{Block, Borders, Clear, List, ListItem},
};
use ratatui_textarea::ScreenCursor;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::CompletionCandidate;

const MAX_VISIBLE: usize = 10;
const MIN_WIDTH: u16 = 20;
const MAX_WIDTH: u16 = 60;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ViewportState {
  pub top_row: u16,
  pub top_col: u16,
}

impl ViewportState {
  pub fn keep_cursor_visible(&mut self, cursor: ScreenCursor, inner: Rect, gutter_width: u16) {
    self.top_row = next_scroll_top(self.top_row, cursor.row as u16, inner.height);
    let cursor_col = cursor.col as u16 + gutter_width;
    self.top_col = next_scroll_top(self.top_col, cursor_col, inner.width);
  }

  pub fn scroll(&mut self, rows: i16, cols: i16) {
    self.top_row = apply_delta(self.top_row, rows);
    self.top_col = apply_delta(self.top_col, cols);
  }
}

pub fn cursor_anchor(
  inner: Rect,
  cursor: ScreenCursor,
  viewport: ViewportState,
  gutter_width: u16,
) -> Option<Position> {
  let row = (cursor.row as u16).checked_sub(viewport.top_row)?;
  let col = (cursor.col as u16 + gutter_width).checked_sub(viewport.top_col)?;
  (row < inner.height && col < inner.width).then_some(Position::new(inner.x + col, inner.y + row))
}

pub fn render_dropdown(
  frame: &mut Frame<'_>,
  editor_area: Rect,
  anchor: Position,
  candidates: &[CompletionCandidate],
  selected: usize,
) -> Option<Rect> {
  if candidates.is_empty() || editor_area.width < 4 || editor_area.height < 3 {
    return None;
  }

  let visible = candidates.len().min(MAX_VISIBLE);
  let height = visible as u16 + 2;
  let label_width = candidates.iter().map(|item| item.label.width()).max().unwrap_or(0);
  let detail_width = candidates
    .iter()
    .map(|item| item.detail.as_deref().unwrap_or(item.kind.label()).width())
    .max()
    .unwrap_or(0);
  let wanted_width = (label_width + detail_width + 6) as u16;
  let max_available = editor_area.width.saturating_sub(1).max(1);
  let min_width = MIN_WIDTH.min(max_available);
  let width = wanted_width.clamp(min_width, MAX_WIDTH.min(max_available));

  let max_x = editor_area.right().saturating_sub(width);
  let x = anchor.x.min(max_x).max(editor_area.x);
  let below = anchor.y.saturating_add(1);
  let y = if below.saturating_add(height) <= editor_area.bottom() {
    below
  } else {
    anchor.y.saturating_sub(height)
  }
  .max(editor_area.y);
  let area = Rect::new(x, y, width, height.min(editor_area.height));
  if area.height < 3 {
    return None;
  }

  let visible_rows = area.height.saturating_sub(2) as usize;
  let selected = selected.min(candidates.len().saturating_sub(1));
  let start = if candidates.len() <= visible_rows {
    0
  } else {
    selected.saturating_sub(visible_rows / 2).min(candidates.len().saturating_sub(visible_rows))
  };
  let content_width = area.width.saturating_sub(4) as usize;
  let items: Vec<ListItem> = candidates
    .iter()
    .enumerate()
    .skip(start)
    .take(visible_rows)
    .map(|(index, candidate)| {
      let detail = candidate.detail.as_deref().unwrap_or(candidate.kind.label());
      let detail_width = detail.width().min(content_width / 2);
      let label_width = content_width.saturating_sub(detail_width + 1);
      let label = truncate(&candidate.label, label_width);
      let detail = truncate(detail, detail_width);
      let padding = " ".repeat(label_width.saturating_sub(label.width()));
      let style = if index == selected {
        Style::default().fg(Color::Black).bg(Color::LightBlue).add_modifier(Modifier::BOLD)
      } else {
        Style::default().fg(Color::White)
      };
      ListItem::new(Line::from(vec![
        Span::styled(format!(" {label}{padding}"), style),
        Span::styled(
          format!(" {detail}"),
          style.fg(if index == selected { Color::Black } else { Color::DarkGray }),
        ),
      ]))
    })
    .collect();

  frame.render_widget(Clear, area);
  frame.render_widget(
    List::new(items).block(
      Block::default()
        .borders(Borders::ALL)
        .title(" suggestions ")
        .border_style(Style::default().fg(Color::LightBlue)),
    ),
    area,
  );
  Some(area)
}

fn truncate(text: &str, width: usize) -> String {
  if text.width() <= width {
    return text.to_owned();
  }
  if width <= 1 {
    return "…".chars().take(width).collect();
  }
  let target = width - 1;
  let mut used = 0;
  let mut result = String::new();
  for ch in text.chars() {
    let char_width = ch.width().unwrap_or(0);
    if used + char_width > target {
      break;
    }
    result.push(ch);
    used += char_width;
  }
  result.push('…');
  result
}

fn next_scroll_top(previous: u16, cursor: u16, length: u16) -> u16 {
  if length == 0 || cursor < previous {
    cursor
  } else if previous.saturating_add(length) <= cursor {
    cursor.saturating_add(1).saturating_sub(length)
  } else {
    previous
  }
}

fn apply_delta(value: u16, delta: i16) -> u16 {
  if delta >= 0 { value.saturating_add(delta as u16) } else { value.saturating_sub(-delta as u16) }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::completion::{CompletionKind, CompletionSource};

  #[test]
  fn anchor_uses_screen_cursor_and_viewport() {
    let inner = Rect::new(1, 1, 40, 5);
    let cursor = ScreenCursor { row: 12, col: 7, char: None, dc: None };
    let anchor = cursor_anchor(inner, cursor, ViewportState { top_row: 10, top_col: 0 }, 3);
    assert_eq!(anchor, Some(Position::new(11, 3)));
  }

  #[test]
  fn dropdown_flips_above_near_bottom() {
    let backend = ratatui::backend::TestBackend::new(50, 15);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    let candidate =
      CompletionCandidate::new("users", CompletionKind::Table, CompletionSource::Database);
    let mut rendered = None;
    terminal
      .draw(|frame| {
        rendered = render_dropdown(
          frame,
          Rect::new(0, 0, 50, 15),
          Position::new(10, 13),
          std::slice::from_ref(&candidate),
          0,
        );
      })
      .unwrap();
    assert!(rendered.unwrap().y < 13);
  }

  #[test]
  fn truncation_uses_terminal_cell_width() {
    assert_eq!(truncate("表名", 3), "表…");
    assert_eq!(truncate("café", 4), "café");
  }
}
