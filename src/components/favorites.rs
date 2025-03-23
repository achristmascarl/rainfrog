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
  search: Option<String>,
  search_focused: bool,
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

  pub fn get_name(&self) -> &str {
    &self.name
  }
}

impl FavoriteEntries {
  pub fn new(favorites_dir: &Path) -> Result<Self> {
    Ok(Self { entries: Self::read_queries(favorites_dir)?, dir: favorites_dir.to_path_buf() })
  }

  pub fn filter(&self, search: Option<String>) -> Vec<&FavoriteEntry> {
    self
      .iter()
      .filter(|entry| {
        if let Some(search) = search.as_ref() {
          entry.name.to_lowercase().contains(search.to_lowercase().trim())
            || entry.query_lines.join("\n").to_lowercase().contains(search.to_lowercase().trim())
        } else {
          true
        }
      })
      .collect::<Vec<&FavoriteEntry>>()
  }

  pub fn iter(&self) -> std::slice::Iter<'_, FavoriteEntry> {
    self.entries.iter()
  }

  pub fn is_empty(&self) -> bool {
    self.entries.is_empty()
  }

  pub fn len(&self) -> usize {
    self.iter().len()
  }

  pub fn delete_entry(&mut self, name: String) {
    if let Some((i, e)) = self.iter().enumerate().find(|(_, entry)| entry.name == name) {
      let entry = self.entries.remove(i);
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
    Favorites {
      command_tx: None,
      config: Config::default(),
      list_state: ListState::default(),
      copied: false,
      search: None,
      search_focused: false,
    }
  }

  pub fn scroll_up(&mut self) {
    let current_selected = self.list_state.selected();
    if let Some(i) = current_selected {
      self.list_state.select(Some(i.saturating_sub(1)));
    }
  }

  pub fn scroll_down(&mut self) {
    let current_selected = self.list_state.selected();
    if let Some(i) = current_selected {
      self.list_state.select(Some(i.saturating_add(1)));
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
        self.scroll_down();
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
    let filtered = app_state.favorites.filter(self.search.clone());
    match key.code {
      KeyCode::Enter if self.search_focused => {
        self.search_focused = false;
        if let Some(search) = &self.search {
          if search.is_empty() {
            self.search = None;
          }
        }
        self.list_state = ListState::default().with_selected(Some(0));
      },
      KeyCode::Char(c) if self.search_focused => {
        if let Some(search) = self.search.as_mut() {
          search.push(c);
          self.list_state = ListState::default();
        }
      },
      KeyCode::Char('/') if !self.search_focused => {
        self.search_focused = true;
        if self.search.is_none() {
          self.search = Some(String::new());
        }
        self.list_state = ListState::default();
      },
      KeyCode::Backspace if self.search_focused => {
        if let Some(search) = self.search.as_mut() {
          search.pop();
          self.list_state = ListState::default();
        }
      },
      KeyCode::Esc => {
        self.search = None;
        self.search_focused = false;
        self.list_state = ListState::default().with_selected(Some(0));
      },
      KeyCode::Down | KeyCode::Char('j') => {
        self.scroll_down();
      },
      KeyCode::Up | KeyCode::Char('k') => {
        self.scroll_up();
      },
      KeyCode::Char('g') => {
        self.list_state.select(Some(0));
      },
      KeyCode::Char('D') => {
        if let Some(i) = current_selected {
          self.command_tx.as_ref().unwrap().send(Action::DeleteFavorite(filtered[i].name.clone()))?;
        }
      },
      KeyCode::Char('y') => {
        if let Some(i) = current_selected {
          self.command_tx.as_ref().unwrap().send(Action::CopyData(filtered[i].query_lines.join("\n")))?;
          self.copied = true;
        }
      },
      KeyCode::Char('G') => self.list_state.select(Some(filtered.len().saturating_sub(1))),
      KeyCode::Char('I') => {
        if let Some(i) = current_selected {
          self.command_tx.as_ref().unwrap().send(Action::FavoriteToEditor(filtered[i].query_lines.clone()))?;
          self.command_tx.as_ref().unwrap().send(Action::FocusEditor)?;
        }
      },
      _ => {},
    };
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

    let filtered_items = app_state.favorites.filter(self.search.clone());
    let filtered_count = filtered_items.len();

    match self.list_state.selected() {
      Some(x) if x > filtered_items.len().saturating_sub(1) => {
        self.list_state.select(Some(0));
      },
      None if !self.search_focused => {
        self.list_state.select(Some(0));
      },
      _ => {},
    };

    let item_lines = filtered_items.into_iter().enumerate().map(|(i, h)| {
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

    let list = List::default()
      .items(item_lines)
      .block(block)
      .highlight_style(Style::default().bold())
      .highlight_symbol(if self.copied { "  " } else { " > " })
      .highlight_spacing(HighlightSpacing::Always);

    // create a space for the search if present
    let mut constraints = vec![Constraint::Percentage(100)];
    if let Some(search) = &self.search {
      constraints.insert(0, Constraint::Length(1));
    }
    let layout = Layout::default().constraints(constraints).direction(Direction::Vertical).split(area);
    if let Some(search) = self.search.as_ref() {
      f.render_widget(
        Text::styled(
          "/ ".to_owned() + search.to_owned().as_str(),
          if !focused {
            Style::new().dim()
          } else if self.search_focused {
            Style::default().fg(Color::Yellow)
          } else {
            Style::default()
          },
        ),
        layout[0],
      )
    }

    f.render_stateful_widget(list, layout[layout.len().saturating_sub(1)], &mut self.list_state);
    let vertical_scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
      .symbols(scrollbar::VERTICAL)
      .style(if focused { Style::default().fg(Color::Green) } else { Style::default() });
    let mut vertical_scrollbar_state =
      ScrollbarState::new(filtered_count.saturating_sub(1)).position(self.list_state.selected().map_or(0, |x| x));
    f.render_stateful_widget(vertical_scrollbar, scrollbar_margin, &mut vertical_scrollbar_state);

    Ok(())
  }
}
