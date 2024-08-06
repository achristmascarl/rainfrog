use std::{
  borrow::BorrowMut,
  collections::HashMap,
  sync::{Arc, Mutex},
  time::Duration,
};

use color_eyre::eyre::Result;
use crossterm::event::{KeyCode, KeyEvent, MouseEventKind};
use indexmap::IndexMap;
use ratatui::{prelude::*, widgets::*};
use serde::{Deserialize, Serialize};
use symbols::scrollbar;
use tokio::sync::mpsc::UnboundedSender;

use super::{Component, Frame};
use crate::{
  action::Action,
  app::{App, AppState},
  config::{Config, KeyBindings},
  database::{get_headers, parse_value, row_to_json, row_to_vec, DbError, Rows},
  focus::Focus,
  tui::Event,
};

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub enum MenuFocus {
  #[default]
  Schema,
  Tables,
}

pub trait SettableTableList<'a> {
  fn set_table_list(&mut self, data: Option<Result<Rows, DbError>>);
}

pub trait MenuComponent<'a>: Component + SettableTableList<'a> {}
impl<'a, T> MenuComponent<'a> for T where T: Component + SettableTableList<'a>
{
}

#[derive(Debug, Clone, Default)]
pub struct Menu {
  command_tx: Option<UnboundedSender<Action>>,
  config: Config,
  table_map: IndexMap<String, Vec<String>>,
  schema_index: usize,
  list_state: ListState,
  menu_focus: MenuFocus,
  search: Option<String>,
  search_focused: bool,
}

impl Menu {
  pub fn new() -> Self {
    Menu {
      command_tx: None,
      config: Config::default(),
      table_map: IndexMap::new(),
      schema_index: 0,
      list_state: ListState::default(),
      menu_focus: MenuFocus::default(),
      search: None,
      search_focused: false,
    }
  }

  pub fn change_focus(&mut self, new_focus: MenuFocus) {
    if self.menu_focus != new_focus && self.table_map.keys().len() > 1 {
      match new_focus {
        MenuFocus::Schema => {
          self.list_state = ListState::default();
        },
        MenuFocus::Tables => {
          self.list_state = ListState::default().with_selected(Some(0));
        },
      }
      self.menu_focus = new_focus;
    }
  }

  pub fn scroll_down(&mut self) {
    match self.menu_focus {
      MenuFocus::Tables => {
        if let Some(i) = self.list_state.selected() {
          let tables = self.table_map.get_index(self.schema_index).unwrap().1.to_owned();
          let filtered_tables: Vec<String> = tables
            .into_iter()
            .filter(|t| {
              if let Some(search) = self.search.as_ref() {
                t.to_lowercase().contains(search.to_lowercase().trim())
              } else {
                true
              }
            })
            .collect();
          self.list_state = ListState::default()
            .with_selected(Some(i.saturating_add(1).clamp(0, filtered_tables.len().saturating_sub(1))));
        }
      },
      MenuFocus::Schema => {
        self.schema_index = self.schema_index.saturating_add(1).clamp(0, self.table_map.keys().len().saturating_sub(1))
      },
    }
  }

  pub fn scroll_up(&mut self) {
    match self.menu_focus {
      MenuFocus::Tables => {
        if let Some(i) = self.list_state.selected() {
          self.list_state = ListState::default().with_selected(Some(i.saturating_sub(1)));
        }
      },
      MenuFocus::Schema => self.schema_index = self.schema_index.saturating_sub(1),
    }
  }

  pub fn scroll_bottom(&mut self) {
    match self.menu_focus {
      MenuFocus::Tables => {
        if let Some(i) = self.list_state.selected() {
          let tables = self.table_map.get_index(self.schema_index).unwrap().1.to_owned();
          let filtered_tables: Vec<String> = tables
            .into_iter()
            .filter(|t| {
              if let Some(search) = self.search.as_ref() {
                t.to_lowercase().contains(search.to_lowercase().trim())
              } else {
                true
              }
            })
            .collect();
          self.list_state = ListState::default().with_selected(Some(filtered_tables.len().saturating_sub(1)));
        }
      },
      MenuFocus::Schema => {
        self.schema_index = self.table_map.keys().len().saturating_sub(1);
      },
    }
  }

  pub fn scroll_top(&mut self) {
    match self.menu_focus {
      MenuFocus::Tables => {
        if let Some(i) = self.list_state.selected() {
          self.list_state = ListState::default().with_selected(Some(0));
        }
      },
      MenuFocus::Schema => self.schema_index = 0,
    }
  }

  pub fn reset_search(&mut self) {
    self.search = None;
    self.search_focused = false;
    self.list_state = ListState::default().with_selected(Some(0));
  }
}

impl<'a> SettableTableList<'a> for Menu {
  fn set_table_list(&mut self, data: Option<Result<Rows, DbError>>) {
    log::info!("setting menu table list");
    self.table_map = IndexMap::new();
    match data {
      Some(Ok(rows)) => {
        rows.0.iter().for_each(|row| {
          let row_as_strings = row_to_vec(row);
          let schema = row_as_strings[0].clone();
          let table = row_as_strings[1].clone();
          if !self.table_map.contains_key(&schema) {
            self.table_map.insert(schema.clone(), vec![]);
          }
          self.table_map.get_mut(&schema).unwrap().push(table.clone());
        });
        log::info!("table map: {:?}", self.table_map);
        if self.table_map.keys().len() == 1 {
          self.menu_focus = MenuFocus::Tables;
          self.list_state = ListState::default().with_selected(Some(0));
        } else {
          self.menu_focus = MenuFocus::Schema;
          self.list_state = ListState::default();
        }
      },
      Some(Err(e)) => {
        log::info!("{}", e);
      },
      None => {},
    }
  }
}

impl Component for Menu {
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
    if app_state.focus != Focus::Menu {
      return Ok(None);
    }
    match mouse.kind {
      MouseEventKind::ScrollDown => self.scroll_down(),
      MouseEventKind::ScrollUp => self.scroll_up(),
      _ => {},
    };
    Ok(None)
  }

  fn handle_key_events(&mut self, key: KeyEvent, app_state: &AppState) -> Result<Option<Action>> {
    if app_state.focus != Focus::Menu {
      return Ok(None);
    }
    match key.code {
      KeyCode::Right => self.change_focus(MenuFocus::Tables),
      KeyCode::Left => self.change_focus(MenuFocus::Schema),
      KeyCode::Down => self.scroll_down(),
      KeyCode::Up => self.scroll_up(),
      KeyCode::Char(c) => {
        if self.search.is_some() && self.search_focused {
          if let Some(search) = self.search.as_mut() {
            search.push(c);
            self.list_state = ListState::default().with_selected(Some(0));
          }
        } else {
          match key.code {
            KeyCode::Char('/') => {
              self.search_focused = true;
              if self.search.is_none() {
                self.search = Some("".to_owned())
              }
            },
            KeyCode::Char('l') => self.change_focus(MenuFocus::Tables),
            KeyCode::Char('h') => self.change_focus(MenuFocus::Schema),
            KeyCode::Char('j') => self.scroll_down(),
            KeyCode::Char('k') => self.scroll_up(),
            KeyCode::Char('g') => self.scroll_top(),
            KeyCode::Char('G') => self.scroll_bottom(),
            KeyCode::Char('R') => self.command_tx.as_ref().unwrap().send(Action::LoadMenu)?,
            _ => {},
          }
        }
      },
      KeyCode::Enter => {
        if self.search.is_some() && self.search_focused {
          self.search_focused = false;
        } else if self.menu_focus == MenuFocus::Schema {
          self.change_focus(MenuFocus::Tables);
        } else if let Some(selected) = self.list_state.selected() {
          let (schema, tables) = self.table_map.get_index(self.schema_index).unwrap();
          let filtered_tables: Vec<String> = tables
            .iter()
            .filter(|t| {
              if let Some(search) = self.search.as_ref() {
                t.to_lowercase().contains(search.to_lowercase().trim())
              } else {
                true
              }
            })
            .cloned()
            .collect();
          self
            .command_tx
            .as_ref()
            .unwrap()
            .send(Action::MenuSelect(schema.clone(), filtered_tables[selected].clone()))?;
        }
      },
      KeyCode::Esc => self.reset_search(),
      KeyCode::Backspace => {
        if self.search.is_some() && self.search_focused {
          if let Some(search) = self.search.as_mut() {
            if !search.is_empty() {
              search.pop();
              self.list_state = ListState::default().with_selected(Some(0));
            } else {
              self.reset_search();
            }
          }
        } else if self.menu_focus == MenuFocus::Tables {
          self.change_focus(MenuFocus::Schema);
        }
      },
      _ => {},
    }
    Ok(None)
  }

  fn draw(&mut self, f: &mut Frame<'_>, area: Rect, app_state: &AppState) -> Result<()> {
    let focused = app_state.focus == Focus::Menu;
    let parent_block = Block::default();
    let stable_keys = self.table_map.keys().enumerate();
    let mut constraints: Vec<Constraint> = stable_keys
      .clone()
      .map(|(i, k)| {
        match i {
          x if x == self.schema_index => Constraint::Min(5),
          _ => Constraint::Length(1),
        }
      })
      .collect();
    if let Some(search) = self.search.as_ref() {
      constraints.insert(0, Constraint::Length(1));
    }
    let layout =
      Layout::default().constraints(constraints).direction(Direction::Vertical).split(parent_block.inner(area));
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
    stable_keys.for_each(|(i, k)| {
      let layout_index = if self.search.is_some() { i + 1 } else { i };
      match i {
        x if x == self.schema_index => {
          let block = Block::default()
            .title(format!(" 󰦄  {} <alt+1> (schema) ", k))
            .borders(Borders::ALL)
            .border_style(if focused && self.menu_focus == MenuFocus::Schema {
              Style::default().fg(Color::Green)
            } else if focused {
              Style::default()
            } else {
              Style::new().dim()
            })
            .padding(Padding { left: 0, right: 1, top: 0, bottom: 0 });
          let block_margin = layout[layout_index].inner(Margin { vertical: 1, horizontal: 0 });
          let tables = self.table_map.get_key_value(k).unwrap().1.clone();
          let filtered_tables: Vec<String> = tables
            .into_iter()
            .filter(|t| {
              if let Some(search) = self.search.as_ref() {
                t.to_lowercase().contains(search.to_lowercase().trim())
              } else {
                true
              }
            })
            .collect();
          let table_length = filtered_tables.len();
          let available_height = block.inner(parent_block.inner(area)).height as usize;
          let list = List::default().items(filtered_tables).block(block).highlight_style(
            Style::default()
              .bg(if focused && !self.search_focused && self.menu_focus == MenuFocus::Tables {
                Color::Green
              } else {
                Color::White
              })
              .fg(Color::DarkGray),
          );
          f.render_stateful_widget(list, layout[layout_index], &mut self.list_state);
          let vertical_scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .symbols(scrollbar::VERTICAL)
            .style(if focused && !self.search_focused && self.menu_focus == MenuFocus::Tables {
              Style::default().fg(Color::Green)
            } else {
              Style::default()
            });
          let mut vertical_scrollbar_state =
            ScrollbarState::new(table_length.saturating_sub(available_height)).position(self.list_state.offset());
          f.render_stateful_widget(vertical_scrollbar, block_margin, &mut vertical_scrollbar_state);
        },
        x if x == self.table_map.keys().len().saturating_sub(1) => {
          f.render_widget(
            Text::styled(
              "└ ".to_owned() + k.to_owned().as_str(),
              if focused { Style::default() } else { Style::new().dim() },
            ),
            layout[layout_index],
          );
        },
        0 => {
          f.render_widget(
            Text::styled(
              "┌ ".to_owned() + k.to_owned().as_str(),
              if focused { Style::default() } else { Style::new().dim() },
            ),
            layout[layout_index],
          );
        },
        _ => {
          f.render_widget(
            Text::styled(
              "├ ".to_owned() + k.to_owned().as_str(),
              if focused { Style::default() } else { Style::new().dim() },
            ),
            layout[layout_index],
          )
        },
      };
    });

    f.render_widget(parent_block, area);
    Ok(())
  }
}
