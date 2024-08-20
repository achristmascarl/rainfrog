use color_eyre::eyre::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseEvent, MouseEventKind};
use ratatui::{prelude::*, symbols::scrollbar, widgets::*};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use tui_textarea::{Input, Key, Scrolling, TextArea};

use super::{Component, Frame};
use crate::{
  action::{Action, MenuPreview},
  app::{App, AppState},
  config::{Config, KeyBindings},
  focus::Focus,
  tui::Event,
};

#[derive(Default)]
pub struct History {
  command_tx: Option<UnboundedSender<Action>>,
  config: Config,
  list_state: ListState,
  copied: bool,
}

impl History {
  pub fn new() -> Self {
    History { command_tx: None, config: Config::default(), list_state: ListState::default(), copied: false }
  }

  pub fn scroll_up(&mut self) {
    let current_selected = self.list_state.selected();
    if let Some(i) = current_selected {
      self.list_state.select(Some(i.saturating_sub(1)));
    }
  }

  pub fn scroll_down(&mut self, item_count: usize) {
    let current_selected = self.list_state.selected();
    if let Some(i) = current_selected {
      self.list_state.select(Some(std::cmp::min(i.saturating_add(1), item_count.saturating_sub(1))));
    }
  }
}

impl Component for History {
  fn register_action_handler(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
    self.command_tx = Some(tx);
    Ok(())
  }

  fn register_config_handler(&mut self, config: Config) -> Result<()> {
    self.config = config;
    Ok(())
  }

  fn handle_mouse_events(&mut self, mouse: MouseEvent, app_state: &AppState) -> Result<Option<Action>> {
    if app_state.focus != Focus::History {
      return Ok(None);
    }
    self.copied = false;
    match mouse.kind {
      MouseEventKind::ScrollDown => {
        self.scroll_down(app_state.history.len());
      },
      MouseEventKind::ScrollUp => {
        self.scroll_up();
      },
      _ => {},
    };
    Ok(None)
  }

  fn handle_key_events(&mut self, key: KeyEvent, app_state: &AppState) -> Result<Option<Action>> {
    if app_state.focus != Focus::History {
      return Ok(None);
    }
    self.copied = false;
    let current_selected = self.list_state.selected();
    if let Some(i) = current_selected {
      match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
          self.scroll_down(app_state.history.len());
        },
        KeyCode::Up | KeyCode::Char('k') => {
          self.scroll_up();
        },
        KeyCode::Char('g') => {
          self.list_state.select(Some(0));
        },
        KeyCode::Char('G') => self.list_state.select(Some(app_state.history.len().saturating_sub(1))),
        KeyCode::Char('I') => {
          self.command_tx.as_ref().unwrap().send(Action::HistoryToEditor(app_state.history[i].query_lines.clone()))?;
          self.command_tx.as_ref().unwrap().send(Action::FocusEditor)?;
        },
        KeyCode::Char('y') => {
          self.command_tx.as_ref().unwrap().send(Action::CopyData(app_state.history[i].query_lines.join("\n")))?;
          self.copied = true;
        },
        KeyCode::Char('D') => {
          self.command_tx.as_ref().unwrap().send(Action::ClearHistory)?;
        },
        _ => {},
      };
    }
    Ok(None)
  }

  fn update(&mut self, action: Action, app_state: &AppState) -> Result<Option<Action>> {
    Ok(None)
  }

  fn draw(&mut self, f: &mut Frame<'_>, area: Rect, app_state: &AppState) -> Result<()> {
    let focused = app_state.focus == Focus::History;
    let block = Block::default().borders(Borders::ALL).border_style(if focused {
      Style::new().green()
    } else {
      Style::new().dim()
    });
    let scrollbar_margin = area.inner(Margin { vertical: 1, horizontal: 0 });

    let items = app_state
      .history
      .iter()
      .enumerate()
      .map(|(i, h)| {
        let selected = self.list_state.selected().map_or(false, |x| i == x);
        let color = if selected && focused { Color::Blue } else { Color::default() };
        let max_lines = 1_usize.max(area.height.saturating_sub(6) as usize);
        let mut lines = h
          .query_lines[0..max_lines.min(h.query_lines.len())]
          .iter()
          .map(|s| Line::from(s.clone()).style(Style::default().fg(color)))
          .collect::<Vec<Line>>();
        if h.query_lines.len() > max_lines {
          lines.push(Line::from(format!("... and {} more lines", h.query_lines.len().saturating_sub(max_lines))).style(Style::default().fg(color)));
        }
        lines.insert(
          0,
          Line::from(format!("{}{}", if self.copied && selected { " copied! - " } else { "" }, h.timestamp))
            .style(if focused { Color::Yellow } else { Color::default() }),
        );
        lines.push(
          Line::from("----------------------------------------------------------------------------------------------------------------------------------------------------------------")
            .style(Style::default().fg(color)),
        );
        ListItem::new(Text::from_iter(lines))
      })
      .collect::<Vec<ListItem>>();

    match self.list_state.selected() {
      Some(x) if x > items.len().saturating_sub(1) => {
        self.list_state.select(Some(0));
      },
      None => {
        self.list_state.select(Some(0));
      },
      _ => {},
    };

    let list = List::default()
      .items(items)
      .block(block)
      .highlight_style(Style::default().bold())
      .highlight_symbol(if self.copied { "  " } else { " > " })
      .highlight_spacing(HighlightSpacing::Always);

    f.render_stateful_widget(list, area, &mut self.list_state);
    let vertical_scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
      .symbols(scrollbar::VERTICAL)
      .style(if focused { Style::default().fg(Color::Green) } else { Style::default() });
    let mut vertical_scrollbar_state = ScrollbarState::new(app_state.history.len().saturating_sub(1))
      .position(self.list_state.selected().map_or(0, |x| x));
    f.render_stateful_widget(vertical_scrollbar, scrollbar_margin, &mut vertical_scrollbar_state);
    Ok(())
  }
}
