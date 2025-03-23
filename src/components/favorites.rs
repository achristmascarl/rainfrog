use std::path::{Path, PathBuf};

use chrono::{format, DateTime, Local, Utc};
use color_eyre::eyre::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseEvent, MouseEventKind};
use ratatui::{prelude::*, symbols::scrollbar, widgets::*};
use serde::{Deserialize, Serialize};
use sqlx::{query, Database, Executor, Pool};
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
pub struct Favorites {
  command_tx: Option<UnboundedSender<Action>>,
  config: Config,
  list_state: ListState,
  copied: bool,
}

pub struct FavoriteEntries {
  dir: PathBuf,
  entries: Vec<FavoriteEntry>,
}

pub struct FavoriteEntry {
  name: String,
  query_lines: Vec<String>,
}

impl FavoriteEntry {
  pub fn path(&self, base: PathBuf) -> PathBuf {
    Self::path_impl(base, self.name.clone())
  }

  pub fn path_impl(mut base: PathBuf, name: String) -> PathBuf {
    base.push(format!("{}.sql", name));
    base
  }
}

impl FavoriteEntries {
  pub fn new(favorites_dir: &Path) -> Result<Self> {
    Ok(Self { entries: Self::read_queries(favorites_dir)?, dir: favorites_dir.to_path_buf() })
  }

  pub fn iter(&self) -> std::slice::Iter<'_, FavoriteEntry> {
    self.entries.iter()
  }

  pub fn is_empty(&self) -> bool {
    self.entries.is_empty()
  }

  pub fn len(&self) -> usize {
    self.entries.len()
  }

  pub fn delete_entry(&mut self, index: usize) {
    if index < self.entries.len() {
      let entry = self.entries.remove(index);
      if let Err(e) = std::fs::remove_file(entry.path(self.dir.clone())) {
        log::error!("failed to delete favorite query from disk: {e}");
      }
    }
  }

  pub fn add_entry(&mut self, name: String, query_lines: Vec<String>) {
    if query_lines.iter().map(|l| l.len()).sum::<usize>() > 0 {
      let creation_time = Local::now();
      let content = query_lines.join("\n");

      match std::fs::write(FavoriteEntry::path_impl(self.dir.clone(), name.clone()), content) {
        Ok(_) => {
          self.entries = Self::read_queries(&self.dir).unwrap_or_else(|e| {
            log::error!("failed to read favorite queries after writing new entry: {e}");
            Vec::new()
          });
        },
        Err(e) => {
          log::error!("failed to create favorite query disk content: {e}");
        },
      }
    }
  }

  fn read_queries(favorites_dir: &Path) -> Result<Vec<FavoriteEntry>> {
    let paths = std::fs::read_dir(favorites_dir)?;

    let mut out = Vec::new();

    for path in paths {
      match path {
        Ok(p) => {
          if let Some(file_name) = p.path().file_name().and_then(|p| p.to_str()) {
            if !file_name.ends_with(".sql") {
              continue;
            }
            let Some(name) = file_name.split('.').next() else {
              continue;
            };
            match std::fs::read_to_string(p.path()) {
              Ok(query_text) => {
                out.push(FavoriteEntry {
                  name: name.to_string(),
                  query_lines: query_text.split('\n').map(|s| s.to_string()).collect(),
                });
              },
              Err(e) => {
                log::error!("failed to read favorite query disk content file_name: '{file_name}' error: {e}");
              },
            };
          }
        },
        Err(e) => {
          log::error!("failed to read favorite query path: {e}");
        },
      };
    }

    Ok(out)
  }
}

impl Favorites {
  pub fn new() -> Self {
    Favorites { command_tx: None, config: Config::default(), list_state: ListState::default(), copied: false }
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

impl<DB: sqlx::Database> Component<DB> for Favorites {
  fn register_action_handler(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
    self.command_tx = Some(tx);
    Ok(())
  }

  fn register_config_handler(&mut self, config: Config) -> Result<()> {
    self.config = config;
    Ok(())
  }

  fn handle_mouse_events(&mut self, mouse: MouseEvent, app_state: &AppState<'_, DB>) -> Result<Option<Action>> {
    if app_state.focus != Focus::Favorites {
      return Ok(None);
    }
    self.copied = false;
    match mouse.kind {
      MouseEventKind::ScrollDown => {
        self.scroll_down(app_state.favorites.entries.len());
      },
      MouseEventKind::ScrollUp => {
        self.scroll_up();
      },
      _ => {},
    };
    Ok(None)
  }

  fn handle_key_events(&mut self, key: KeyEvent, app_state: &AppState<'_, DB>) -> Result<Option<Action>> {
    if app_state.focus != Focus::Favorites {
      return Ok(None);
    }
    self.copied = false;
    let current_selected = self.list_state.selected();
    if let Some(i) = current_selected {
      match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
          self.scroll_down(app_state.favorites.len());
        },
        KeyCode::Up | KeyCode::Char('k') => {
          self.scroll_up();
        },
        KeyCode::Char('g') => {
          self.list_state.select(Some(0));
        },
        KeyCode::Char('D') => {
          self.command_tx.as_ref().unwrap().send(Action::DeleteFavorite(i))?;
        },
        KeyCode::Char('y') => {
          self
            .command_tx
            .as_ref()
            .unwrap()
            .send(Action::CopyData(app_state.favorites.entries[i].query_lines.join("\n")))?;
          self.copied = true;
        },
        KeyCode::Char('G') => self.list_state.select(Some(app_state.favorites.len().saturating_sub(1))),
        KeyCode::Char('I') => {
          self
            .command_tx
            .as_ref()
            .unwrap()
            .send(Action::FavoriteToEditor(app_state.favorites.entries[i].query_lines.clone()))?;
          self.command_tx.as_ref().unwrap().send(Action::FocusEditor)?;
        },
        _ => {},
      };
    }
    Ok(None)
  }

  fn update(&mut self, action: Action, app_state: &AppState<'_, DB>) -> Result<Option<Action>> {
    Ok(None)
  }

  fn draw(&mut self, f: &mut Frame<'_>, area: Rect, app_state: &AppState<'_, DB>) -> Result<()> {
    let focused = app_state.focus == Focus::Favorites;
    let block = Block::default().borders(Borders::ALL).border_style(if focused {
      Style::new().green()
    } else {
      Style::new().dim()
    });

    let scrollbar_margin = area.inner(Margin { vertical: 1, horizontal: 0 });

    let items = app_state
      .favorites
      .iter()
      .enumerate()
      .map(|(i, h)| {
        let selected = self.list_state.selected() == Some(i);
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
          Line::from(format!("{}{}", if self.copied && selected { " copied! - " } else { "" }, h.name))
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
    let mut vertical_scrollbar_state = ScrollbarState::new(app_state.favorites.len().saturating_sub(1))
      .position(self.list_state.selected().map_or(0, |x| x));
    f.render_stateful_widget(vertical_scrollbar, scrollbar_margin, &mut vertical_scrollbar_state);

    Ok(())
  }
}
