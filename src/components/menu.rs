use color_eyre::eyre::Result;
use crossterm::event::{KeyCode, KeyEvent, MouseEventKind};
use indexmap::IndexMap;
use ratatui::{prelude::*, widgets::*};
use symbols::scrollbar;
use tokio::sync::mpsc::UnboundedSender;

use super::{Component, Frame};
use crate::{
  action::{Action, MenuItemKind, MenuPreview, MenuTarget},
  app::AppState,
  config::Config,
  database::Rows,
  focus::Focus,
};

#[derive(Debug, Clone)]
struct MenuViewItem {
  name: String,
  materialized: bool,
}

#[derive(Debug, Clone, Default)]
struct MenuSchemaItems {
  tables: Vec<String>,
  views: Vec<MenuViewItem>,
}

#[derive(Debug, Clone)]
struct MenuItem {
  name: String,
  kind: MenuItemKind,
}

#[derive(Debug, Clone)]
enum MenuEntry {
  Header(String),
  Item(MenuItem),
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub enum MenuFocus {
  #[default]
  Schema,
  Tables,
}

pub trait SettableTableList<'a> {
  fn set_table_list(&mut self, data: Option<Result<Rows>>);
}

pub trait MenuComponent<'a>: Component + SettableTableList<'a> {}

impl<'a, T> MenuComponent<'a> for T where T: Component + SettableTableList<'a> {}

#[derive(Debug, Clone, Default)]
pub struct Menu {
  command_tx: Option<UnboundedSender<Action>>,
  config: Config,
  table_map: IndexMap<String, MenuSchemaItems>,
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
          let entries = self.filtered_entries();
          self.list_state = ListState::default().with_selected(Self::first_selectable_index(&entries));
        },
      }
      self.menu_focus = new_focus;
    }
  }

  pub fn scroll_down(&mut self) {
    match self.menu_focus {
      MenuFocus::Tables => {
        let entries = self.filtered_entries();
        if entries.is_empty() {
          self.list_state = ListState::default();
          return;
        }
        let next = match self.list_state.selected() {
          Some(i) => Self::next_selectable_index(&entries, i).or_else(|| Self::first_selectable_index(&entries)),
          None => Self::first_selectable_index(&entries),
        };
        self.list_state = ListState::default().with_selected(next);
      },
      MenuFocus::Schema => {
        self.schema_index = self.schema_index.saturating_add(1).clamp(0, self.table_map.keys().len().saturating_sub(1))
      },
    }
  }

  pub fn scroll_up(&mut self) {
    match self.menu_focus {
      MenuFocus::Tables => {
        let entries = self.filtered_entries();
        if entries.is_empty() {
          self.list_state = ListState::default();
          return;
        }
        let prev = match self.list_state.selected() {
          Some(i) => Self::previous_selectable_index(&entries, i).or_else(|| Self::last_selectable_index(&entries)),
          None => Self::last_selectable_index(&entries),
        };
        self.list_state = ListState::default().with_selected(prev);
      },
      MenuFocus::Schema => self.schema_index = self.schema_index.saturating_sub(1),
    }
  }

  pub fn scroll_bottom(&mut self) {
    match self.menu_focus {
      MenuFocus::Tables => {
        let entries = self.filtered_entries();
        let last = Self::last_selectable_index(&entries);
        self.list_state = ListState::default().with_selected(last);
      },
      MenuFocus::Schema => {
        self.schema_index = self.table_map.keys().len().saturating_sub(1);
      },
    }
  }

  pub fn scroll_top(&mut self) {
    match self.menu_focus {
      MenuFocus::Tables => {
        let entries = self.filtered_entries();
        let first = Self::first_selectable_index(&entries);
        self.list_state = ListState::default().with_selected(first);
      },
      MenuFocus::Schema => self.schema_index = 0,
    }
  }

  pub fn reset_search(&mut self) {
    self.search = None;
    self.search_focused = false;
    let entries = self.filtered_entries();
    self.list_state = ListState::default().with_selected(Self::first_selectable_index(&entries));
  }

  fn filtered_entries(&self) -> Vec<MenuEntry> {
    let Some((_, items)) = self.table_map.get_index(self.schema_index) else {
      return vec![];
    };
    let matches_search = |name: &str, search: &Option<String>| {
      if let Some(search) = search.as_ref() { name.to_lowercase().contains(search.to_lowercase().trim()) } else { true }
    };

    let tables: Vec<MenuEntry> = items
      .tables
      .iter()
      .filter(|t| matches_search(t.as_str(), &self.search))
      .cloned()
      .map(|name| MenuEntry::Item(MenuItem { name, kind: MenuItemKind::Table }))
      .collect();

    let views: Vec<MenuEntry> = items
      .views
      .iter()
      .filter(|v| matches_search(v.name.as_str(), &self.search))
      .map(|v| {
        MenuEntry::Item(MenuItem { name: v.name.clone(), kind: MenuItemKind::View { materialized: v.materialized } })
      })
      .collect();

    let mut entries = Vec::new();
    if !tables.is_empty() {
      entries.push(MenuEntry::Header("Tables".to_owned()));
      entries.extend(tables);
    }
    if !views.is_empty() {
      entries.push(MenuEntry::Header("Views".to_owned()));
      entries.extend(views);
    }
    entries
  }

  fn first_selectable_index(entries: &[MenuEntry]) -> Option<usize> {
    entries.iter().position(|entry| matches!(entry, MenuEntry::Item(_)))
  }

  fn last_selectable_index(entries: &[MenuEntry]) -> Option<usize> {
    entries.iter().rposition(|entry| matches!(entry, MenuEntry::Item(_)))
  }

  fn next_selectable_index(entries: &[MenuEntry], current: usize) -> Option<usize> {
    entries
      .iter()
      .enumerate()
      .skip(current + 1)
      .find_map(|(index, entry)| if matches!(entry, MenuEntry::Item(_)) { Some(index) } else { None })
  }

  fn previous_selectable_index(entries: &[MenuEntry], current: usize) -> Option<usize> {
    entries
      .iter()
      .enumerate()
      .take(current)
      .rfind(|(_, entry)| matches!(entry, MenuEntry::Item(_)))
      .map(|(index, _)| index)
  }

  fn selected_item(&self) -> Option<MenuItem> {
    let entries = self.filtered_entries();
    let selected = self.list_state.selected()?;
    match entries.get(selected) {
      Some(MenuEntry::Item(item)) => Some(item.clone()),
      _ => None,
    }
  }
}

impl SettableTableList<'_> for Menu {
  fn set_table_list(&mut self, data: Option<Result<Rows>>) {
    log::info!("setting menu table list");
    self.table_map = IndexMap::new();
    match data {
      Some(Ok(rows)) => {
        rows.rows.iter().for_each(|row| {
          let schema = row.get(0).cloned().unwrap_or_default();
          let name = row.get(1).cloned().unwrap_or_default();
          let kind = row.get(2).map(|value| value.to_lowercase()).unwrap_or_else(|| "table".to_owned());
          let entry = self.table_map.entry(schema.clone()).or_insert_with(MenuSchemaItems::default);
          match kind.as_str() {
            "view" => entry.views.push(MenuViewItem { name, materialized: false }),
            "materialized_view" | "materialized view" | "mview" => {
              entry.views.push(MenuViewItem { name, materialized: true })
            },
            _ => entry.tables.push(name),
          }
        });
        if self.table_map.keys().len() == 1 {
          self.menu_focus = MenuFocus::Tables;
          let entries = self.filtered_entries();
          self.list_state = ListState::default().with_selected(Self::first_selectable_index(&entries));
        } else {
          self.menu_focus = MenuFocus::Schema;
          self.list_state = ListState::default();
        }
      },
      Some(Err(e)) => {
        log::error!("{e}");
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
            let entries = self.filtered_entries();
            self.list_state = ListState::default().with_selected(Self::first_selectable_index(&entries));
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
            KeyCode::Char('1') | KeyCode::Char('2') | KeyCode::Char('3') | KeyCode::Char('4') => {
              if let Some(item) = self.selected_item()
                && let Some((schema, _)) = self.table_map.get_index(self.schema_index)
              {
                let preview = match (key.code, item.kind.clone()) {
                  (KeyCode::Char('1'), _) => Some(MenuPreview::Columns),
                  (KeyCode::Char('2'), MenuItemKind::View { .. }) => Some(MenuPreview::Definition),
                  (KeyCode::Char('2'), MenuItemKind::Table) => Some(MenuPreview::Constraints),
                  (KeyCode::Char('3'), MenuItemKind::Table) => Some(MenuPreview::Indexes),
                  (KeyCode::Char('4'), MenuItemKind::Table) => Some(MenuPreview::Policies),
                  _ => None,
                };
                if let Some(preview) = preview {
                  self.command_tx.as_ref().unwrap().send(Action::MenuPreview(
                    preview,
                    MenuTarget { schema: schema.clone(), name: item.name.clone(), kind: item.kind.clone() },
                  ))?;
                }
              }
            },
            _ => {},
          }
        }
      },
      KeyCode::Enter => {
        if self.search.is_some() && self.search_focused {
          self.search_focused = false;
        } else if self.menu_focus == MenuFocus::Schema {
          self.change_focus(MenuFocus::Tables);
        } else if let Some(item) = self.selected_item()
          && let Some((schema, _)) = self.table_map.get_index(self.schema_index)
        {
          self.command_tx.as_ref().unwrap().send(Action::MenuPreview(
            MenuPreview::Rows,
            MenuTarget { schema: schema.clone(), name: item.name.clone(), kind: item.kind.clone() },
          ))?;
        }
      },
      KeyCode::Esc => self.reset_search(),
      KeyCode::Backspace => {
        if self.search.is_some() && self.search_focused {
          if let Some(search) = self.search.as_mut() {
            if !search.is_empty() {
              search.pop();
              let entries = self.filtered_entries();
              self.list_state = ListState::default().with_selected(Self::first_selectable_index(&entries));
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
    let schema_keys: Vec<String> = self.table_map.keys().cloned().collect();
    let mut constraints: Vec<Constraint> = schema_keys
      .iter()
      .enumerate()
      .map(|(i, _)| match i {
        x if x == self.schema_index => Constraint::Min(5),
        _ => Constraint::Length(1),
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
    schema_keys.iter().enumerate().for_each(|(i, k)| {
      let layout_index = if self.search.is_some() { i + 1 } else { i };
      match i {
        x if x == self.schema_index => {
          let block = Block::default()
            .title(format!(" 󰦄  {k} <alt+1> (schema) "))
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
          let entries = self.filtered_entries();
          let entry_length = entries.len();
          let available_height = block.inner(parent_block.inner(area)).height as usize;
          let selected_index = self.list_state.selected();
          let entries_items: Vec<ListItem> = entries
            .iter()
            .enumerate()
            .map(|(i, entry)| match entry {
              MenuEntry::Header(title) => {
                ListItem::new(Text::styled(format!("─ {title}"), Style::default().fg(Color::DarkGray)))
              },
              MenuEntry::Item(item) => {
                let display_name = match item.kind {
                  MenuItemKind::View { materialized: true } => format!(" {} (materialized)", item.name),
                  _ => " ".to_owned() + &item.name.clone(),
                };
                let is_selected = selected_index == Some(i);
                if is_selected && focused && !self.search_focused {
                  match item.kind {
                    MenuItemKind::Table => ListItem::new(Text::from(vec![
                      Line::from(display_name),
                      Line::from(if app_state.query_task_running { " ├[...] rows" } else { " ├[<enter>] rows" }),
                      Line::from(if app_state.query_task_running { " ├[...] columns" } else { " ├[1] columns" }),
                      Line::from(if app_state.query_task_running {
                        " ├[...] constraints"
                      } else {
                        " ├[2] constraints"
                      }),
                      Line::from(if app_state.query_task_running { " ├[...] indexes" } else { " ├[3] indexes" }),
                      Line::from(if app_state.query_task_running {
                        " └[...] rls policies"
                      } else {
                        " └[4] rls policies"
                      }),
                    ])),
                    MenuItemKind::View { .. } => ListItem::new(Text::from(vec![
                      Line::from(display_name),
                      Line::from(if app_state.query_task_running { " ├[...] rows" } else { " ├[<enter>] rows" }),
                      Line::from(if app_state.query_task_running { " ├[...] columns" } else { " ├[1] columns" }),
                      Line::from(if app_state.query_task_running {
                        "  └[...] schema definition"
                      } else {
                        " └[2] schema definition"
                      }),
                    ])),
                  }
                } else {
                  ListItem::new(display_name)
                }
              },
            })
            .collect();
          let list = List::default().items(entries_items).block(block).highlight_style(
            Style::default()
              .fg(if focused && !self.search_focused && self.menu_focus == MenuFocus::Tables {
                Color::Green
              } else {
                Color::Gray
              })
              .add_modifier(if focused { Modifier::BOLD } else { Modifier::REVERSED }),
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
            ScrollbarState::new(entry_length.saturating_sub(available_height)).position(self.list_state.offset());
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
        _ => f.render_widget(
          Text::styled(
            "├ ".to_owned() + k.to_owned().as_str(),
            if focused { Style::default() } else { Style::new().dim() },
          ),
          layout[layout_index],
        ),
      };
    });

    f.render_widget(parent_block, area);
    Ok(())
  }
}
