#[cfg(not(feature = "termux"))]
use arboard::Clipboard;
use color_eyre::eyre::{Result, eyre};
use crossterm::event::{KeyEvent, MouseEvent, MouseEventKind};
use ratatui::{
  Frame,
  layout::{Constraint, Direction, Layout, Position},
  prelude::Rect,
  style::{Color, Style, Stylize},
  text::Line,
  widgets::{Block, Borders, Clear, Padding, Paragraph, Tabs, Wrap},
};
use sqlparser::ast::Statement;
use strum::IntoEnumIterator;
use tokio::sync::mpsc::{self};

use crate::{
  action::{Action, ExportFormat, MenuPreview},
  cli::{Cli, Driver},
  components::{
    Component, ComponentImpls,
    data::{Data, DataComponent},
    editor::Editor,
    favorites::{FavoriteEntries, Favorites},
    history::History,
    menu::{Menu, MenuComponent},
  },
  config::Config,
  database::{self, Database, DbTaskResult, ExecutionType, Rows},
  focus::Focus,
  popups::{
    PopUp, PopUpPayload, confirm_bypass::ConfirmBypass, confirm_export::ConfirmExport, confirm_query::ConfirmQuery,
    confirm_tx::ConfirmTx, exporting::Exporting, name_favorite::NameFavorite,
  },
  tui,
  ui::center,
};

pub struct HistoryEntry {
  pub query_lines: Vec<String>,
  pub timestamp: chrono::DateTime<chrono::Local>,
}

pub struct AppState {
  pub focus: Focus,
  pub history: Vec<HistoryEntry>,
  pub favorites: FavoriteEntries,
  pub last_query_start: Option<chrono::DateTime<chrono::Utc>>,
  pub last_query_end: Option<chrono::DateTime<chrono::Utc>>,
  pub query_task_running: bool,
}

pub struct Components<'a> {
  pub menu: Box<dyn MenuComponent<'a>>,
  pub editor: Box<dyn Component>,
  pub history: Box<dyn Component>,
  pub data: Box<dyn DataComponent<'a>>,
  pub favorites: Box<dyn Component>,
}

pub struct App {
  pub mouse_mode_override: Option<bool>,
  pub config: Config,
  pub components: Components<'static>,
  pub should_quit: bool,
  pub last_tick_key_events: Vec<KeyEvent>,
  pub last_frame_mouse_event: Option<MouseEvent>,
  pub state: AppState,
  last_focused_tab: Focus,
  last_focused_component: Focus,
  popup: Option<Box<dyn PopUp>>,
}

impl App {
  pub fn new(mouse_mode_override: Option<bool>, config: Config) -> Result<Self> {
    let focus = Focus::Menu;
    let menu = Menu::new();
    let editor = Editor::new();
    let history = History::new();
    let data = Data::new();
    let favorites = Favorites::new();
    let favorite_entries = FavoriteEntries::new(&config.config._favorites_dir)?;

    Ok(Self {
      components: Components {
        menu: Box::new(menu),
        editor: Box::new(editor),
        history: Box::new(history),
        data: Box::new(data),
        favorites: Box::new(favorites),
      },
      should_quit: false,
      mouse_mode_override,
      config,
      last_tick_key_events: Vec::new(),
      last_frame_mouse_event: None,
      state: AppState {
        focus,
        history: vec![],
        last_query_start: None,
        last_query_end: None,
        favorites: favorite_entries,
        query_task_running: false,
      },
      last_focused_tab: Focus::Editor,
      last_focused_component: focus,
      popup: None,
    })
  }

  fn add_to_history(&mut self, query_lines: Vec<String>) {
    self.state.history.insert(0, HistoryEntry { query_lines, timestamp: chrono::Local::now() });
    if self.state.history.len() > 50 {
      self.state.history.pop();
    }
  }

  fn clear_history(&mut self) {
    self.state.history = vec![];
  }

  fn set_focus(&mut self, focus: Focus) {
    self.state.focus = focus;
    if focus != Focus::PopUp {
      self.popup = None;
      self.last_focused_component = focus;
    }
    if focus == Focus::Editor || focus == Focus::History || focus == Focus::Favorites {
      self.last_focused_tab = focus;
    }
  }

  fn set_popup(&mut self, popup: Box<dyn PopUp>) {
    self.popup = Some(popup);
    self.set_focus(Focus::PopUp);
  }

  fn last_focused_tab(&mut self) {
    match self.last_focused_tab {
      Focus::Editor => self.set_focus(Focus::Editor),
      Focus::History => self.set_focus(Focus::History),
      Focus::Favorites => self.set_focus(Focus::Favorites),
      _ => {},
    }
  }

  fn last_focused_component(&mut self) {
    match self.last_focused_component {
      Focus::Menu => self.set_focus(Focus::Menu),
      Focus::Editor => self.set_focus(Focus::Editor),
      Focus::Data => self.set_focus(Focus::Data),
      Focus::History => self.set_focus(Focus::History),
      Focus::Favorites => self.set_focus(Focus::Favorites),
      Focus::PopUp => {},
    }
  }

  pub async fn run(&mut self, driver: Driver, args: Cli) -> Result<()> {
    let mut database: Box<dyn Database> = match driver {
      Driver::Postgres => Box::new(database::PostgresDriver::new()),
      Driver::MySql => Box::new(database::MySqlDriver::new()),
      Driver::Sqlite => Box::new(database::SqliteDriver::new()),
      Driver::Oracle => Box::new(database::OracleDriver::new()),
      #[cfg(not(feature = "musl"))]
      Driver::DuckDb => Box::new(database::DuckDbDriver::new()),
    };
    database.init(args).await?;
    let (action_tx, mut action_rx) = mpsc::unbounded_channel();
    log::info!("{driver:?}");

    let mut tui = tui::Tui::new()?.mouse(self.mouse_mode_override.or(self.config.settings.mouse_mode));
    tui.enter()?;

    #[allow(unused_mut)]
    #[cfg(not(feature = "termux"))]
    let mut clipboard = Clipboard::new();

    self.components.menu.register_action_handler(action_tx.clone())?;
    self.components.editor.register_action_handler(action_tx.clone())?;
    self.components.history.register_action_handler(action_tx.clone())?;
    self.components.data.register_action_handler(action_tx.clone())?;
    self.components.favorites.register_action_handler(action_tx.clone())?;

    self.components.menu.register_config_handler(self.config.clone())?;
    self.components.editor.register_config_handler(self.config.clone())?;
    self.components.history.register_config_handler(self.config.clone())?;
    self.components.data.register_config_handler(self.config.clone())?;
    self.components.favorites.register_config_handler(self.config.clone())?;

    let size = tui.size()?;
    self.components.menu.init(Rect { width: size.width, height: size.height, x: 0, y: 0 })?;
    self.components.editor.init(Rect { width: size.width, height: size.height, x: 0, y: 0 })?;
    self.components.history.init(Rect { width: size.width, height: size.height, x: 0, y: 0 })?;
    self.components.data.init(Rect { width: size.width, height: size.height, x: 0, y: 0 })?;
    self.components.favorites.init(Rect { width: size.width, height: size.height, x: 0, y: 0 })?;

    action_tx.send(Action::LoadMenu)?;

    loop {
      if self.popup.is_some() {
        self.set_focus(Focus::PopUp);
      }
      match database.get_query_results().await? {
        DbTaskResult::Finished(results) => {
          self.components.data.set_data_state(Some(results.results), results.statement_type);
          self.state.last_query_end = Some(chrono::Utc::now());
          self.state.query_task_running = false;
        },
        DbTaskResult::ConfirmTx(rows_affected, statement) => {
          self.state.last_query_end = Some(chrono::Utc::now());
          self.set_popup(Box::new(ConfirmTx::new(rows_affected, statement)));
          self.state.query_task_running = true;
        },
        DbTaskResult::Pending => {
          self.state.query_task_running = true;
        },
        DbTaskResult::NoTask => {
          self.state.query_task_running = false;
        },
      }
      if let Some(e) = tui.next().await {
        let mut event_consumed = false;
        match e {
          tui::Event::Quit => action_tx.send(Action::Quit)?,
          tui::Event::Tick => action_tx.send(Action::Tick)?,
          tui::Event::Render => action_tx.send(Action::Render)?,
          tui::Event::Resize(x, y) => action_tx.send(Action::Resize(x, y))?,
          tui::Event::Mouse(event) => self.last_frame_mouse_event = Some(event),
          tui::Event::Key(key) => {
            if let Some(keymap) = self.config.keybindings.get(&self.state.focus) {
              if let Some(action) = keymap.get(&vec![key]) {
                log::info!("Got action: {action:?}");
                action_tx.send(action.clone())?;
                event_consumed = true;
              } else if let Some(popup) = &mut self.popup {
                // popup captures all inputs. if it returns a payload, that means
                // it is finished and should be closed
                let payload = popup.handle_key_events(key, &mut self.state)?;
                match payload {
                  Some(PopUpPayload::SetDataTable(result, statement)) => {
                    self.components.data.set_data_state(result, statement);
                    self.set_focus(Focus::Editor);
                  },
                  Some(PopUpPayload::ConfirmQuery(query)) => {
                    action_tx.send(Action::Query(vec![query], true, false))?;
                    self.set_focus(Focus::Editor);
                  },
                  Some(PopUpPayload::ConfirmBypass(query)) => {
                    action_tx.send(Action::Query(vec![query], true, true))?;
                    self.set_focus(Focus::Editor);
                  },
                  Some(PopUpPayload::ConfirmExport(confirmed)) => {
                    if confirmed {
                      action_tx.send(Action::ExportData(ExportFormat::CSV))?;
                      self.set_popup(Box::new(Exporting::new()));
                    } else {
                      self.set_focus(Focus::Data);
                    }
                  },
                  Some(PopUpPayload::Cancel) => {
                    self.last_focused_component();
                  },
                  Some(PopUpPayload::NamedFavorite(name, query_lines)) => {
                    self.state.favorites.add_entry(name, query_lines);
                    self.set_focus(Focus::Editor);
                  },
                  Some(PopUpPayload::CommitTx) => {
                    let response = database.commit_tx().await?;
                    self.state.last_query_end = Some(chrono::Utc::now());
                    if let Some(results) = response {
                      self.components.data.set_data_state(Some(results.results), results.statement_type);
                      self.set_focus(Focus::Editor);
                    }
                  },
                  Some(PopUpPayload::RollbackTx) => {
                    database.rollback_tx().await?;
                    self.state.last_query_end = Some(chrono::Utc::now());
                    self.components.data.set_data_state(
                      Some(Ok(Rows { headers: vec![], rows: vec![], rows_affected: None })),
                      Some(Statement::Rollback { chain: false, savepoint: None }),
                    );
                    self.set_focus(Focus::Editor);
                  },
                  None => {},
                }
                event_consumed = true;
              } else {
                // If the key was not handled as a single key action,
                // then consider it for multi-key combinations.
                self.last_tick_key_events.push(key);

                // Check for multi-key combinations
                if let Some(action) = keymap.get(&self.last_tick_key_events) {
                  log::info!("Got action: {action:?}");
                  action_tx.send(action.clone())?;
                  event_consumed = true;
                }
              }
            }
          },
          _ => {},
        }
        if !event_consumed {
          for i in ComponentImpls::iter() {
            let action = match i {
              ComponentImpls::Menu => {
                self.components.menu.handle_events(Some(e.clone()), self.last_tick_key_events.clone(), &self.state)?
              },
              ComponentImpls::Editor => {
                self.components.editor.handle_events(Some(e.clone()), self.last_tick_key_events.clone(), &self.state)?
              },
              ComponentImpls::History => self.components.history.handle_events(
                Some(e.clone()),
                self.last_tick_key_events.clone(),
                &self.state,
              )?,
              ComponentImpls::Data => {
                self.components.data.handle_events(Some(e.clone()), self.last_tick_key_events.clone(), &self.state)?
              },
              ComponentImpls::Favorites => self.components.favorites.handle_events(
                Some(e.clone()),
                self.last_tick_key_events.clone(),
                &self.state,
              )?,
            };
            if let Some(action) = action {
              action_tx.send(action)?;
            }
          }
        }
      }

      while let Ok(action) = action_rx.try_recv() {
        if action != Action::Tick && action != Action::Render {
          log::debug!("{action:?}");
        }
        let action_consumed = false;
        match &action {
          Action::Tick => {
            self.last_tick_key_events.drain(..);
          },
          Action::Quit => self.should_quit = true,
          Action::Resize(w, h) => {
            tui.resize(Rect::new(0, 0, *w, *h))?;
            tui.draw(|f| {
              self.draw_layout(f, action_tx.clone()).expect("Couldn't draw layout");
            })?;
          },
          Action::Render => {
            tui.draw(|f| {
              self.draw_layout(f, action_tx.clone()).expect("Couldn't draw layout");
            })?;
            self.last_frame_mouse_event = None;
          },
          Action::FocusMenu => self.set_focus(Focus::Menu),
          Action::FocusEditor => self.set_focus(Focus::Editor),
          Action::FocusData => self.set_focus(Focus::Data),
          Action::FocusHistory => self.set_focus(Focus::History),
          Action::FocusFavorites => self.set_focus(Focus::Favorites),
          Action::CycleFocusForwards => match self.state.focus {
            Focus::Menu => self.set_focus(Focus::Editor),
            Focus::Editor => self.set_focus(Focus::Data),
            Focus::Data => self.set_focus(Focus::History),
            Focus::History => self.set_focus(Focus::Favorites),
            Focus::Favorites => self.set_focus(Focus::Menu),
            Focus::PopUp => {},
          },
          Action::CycleFocusBackwards => match self.state.focus {
            Focus::History => self.set_focus(Focus::Data),
            Focus::Data => self.set_focus(Focus::Editor),
            Focus::Editor => self.set_focus(Focus::Menu),
            Focus::Menu => self.set_focus(Focus::Favorites),
            Focus::Favorites => self.set_focus(Focus::History),
            Focus::PopUp => {},
          },
          Action::LoadMenu => {
            let rows = database.load_menu().await;
            self.components.menu.set_table_list(Some(rows));
          },
          Action::Query(query_lines, confirmed, bypass) => 'query_action: {
            let query_string = query_lines.clone().join(" \n");
            if query_string.is_empty() {
              break 'query_action;
            }
            self.add_to_history(query_lines.clone());
            if *bypass && !confirmed {
              log::warn!("Bypassing parser");
              self.set_popup(Box::new(ConfirmBypass::new(query_string.clone())));
              break 'query_action;
            }
            let execution_info = match *bypass && *confirmed {
              true => Ok((ExecutionType::Normal, None)),
              false => database::get_execution_type(query_string.clone(), *confirmed, driver),
            };
            match execution_info {
              Ok((ExecutionType::Transaction, _)) => {
                self.components.data.set_loading();
                database.start_tx(query_string).await?;
                self.state.last_query_start = Some(chrono::Utc::now());
                self.state.last_query_end = None;
              },
              Ok((ExecutionType::Confirm, Some(statement_type))) => {
                self.set_popup(Box::new(ConfirmQuery::new(query_string.clone(), statement_type)));
              },
              Ok((ExecutionType::Normal, _)) => {
                self.components.data.set_loading();
                database.start_query(query_string, *bypass).await?;
                self.state.last_query_start = Some(chrono::Utc::now());
                self.state.last_query_end = None;
              },
              Err(e) => self.components.data.set_data_state(Some(Err(e)), None),
              _ => self.components.data.set_data_state(Some(Err(eyre!("Missing statement type but not bypass"))), None),
            }
          },
          Action::AbortQuery => match database.abort_query().await {
            Ok(true) => {
              self.components.data.set_cancelled();
              self.state.last_query_end = Some(chrono::Utc::now());
            },
            Ok(false) => {},
            Err(e) => {
              self.components.data.set_data_state(Some(Err(e)), None);
            },
          },
          Action::MenuPreview(preview_type, schema, table) => {
            let preview_query = match preview_type {
              MenuPreview::Rows => database.preview_rows_query(schema, table),
              MenuPreview::Columns => database.preview_columns_query(schema, table),
              MenuPreview::Constraints => database.preview_constraints_query(schema, table),
              MenuPreview::Indexes => database.preview_indexes_query(schema, table),
              MenuPreview::Policies => database.preview_policies_query(schema, table),
            };
            action_tx.send(Action::QueryToEditor(vec![preview_query.clone()]))?;
            action_tx.send(Action::FocusEditor)?;
            action_tx.send(Action::FocusMenu)?;
            action_tx.send(Action::Query(vec![preview_query.clone()], false, false))?;
          },

          Action::RequestSaveFavorite(query_lines) => {
            self.set_popup(Box::new(NameFavorite::new(
              self.state.favorites.iter().map(|f| f.get_name().to_string()).collect(),
              query_lines.clone(),
            )));
          },
          Action::DeleteFavorite(name) => {
            self.state.favorites.delete_entry(name.clone());
          },
          Action::ClearHistory => {
            self.clear_history();
          },
          Action::CopyData(data) => {
            #[cfg(not(feature = "termux"))]
            {
              clipboard.as_mut().map_or_else(
                |e| {
                  log::error!("{e:?}");
                },
                |clipboard| {
                  clipboard.set_text(data).unwrap_or_else(|e| {
                    log::error!("{e:?}");
                  })
                },
              );
            }
          },
          Action::RequestExportData(row_count) => {
            self.set_popup(Box::new(ConfirmExport::new(*row_count)));
          },
          Action::ExportDataFinished => {
            self.set_focus(Focus::Data);
          },
          _ => {},
        }
        if !action_consumed {
          for i in ComponentImpls::iter() {
            let action = match i {
              ComponentImpls::Menu => self.components.menu.update(action.clone(), &self.state)?,
              ComponentImpls::Editor => self.components.editor.update(action.clone(), &self.state)?,
              ComponentImpls::History => self.components.history.update(action.clone(), &self.state)?,
              ComponentImpls::Data => self.components.data.update(action.clone(), &self.state)?,
              ComponentImpls::Favorites => self.components.favorites.update(action.clone(), &self.state)?,
            };
            if let Some(action) = action {
              log::info!("{action:?}");
              action_tx.send(action)?;
            }
          }
        }
      }

      if self.last_frame_mouse_event.is_some() {
        tui.draw(|f| {
          self.draw_layout(f, action_tx.clone()).expect("Couldn't draw layout");
        })?;
      }
      if self.should_quit {
        database.abort_query().await?;
        tui.stop()?;
        break;
      }
    }
    tui.exit()?;
    Ok(())
  }

  fn draw_layout(&mut self, f: &mut Frame, action_tx: mpsc::UnboundedSender<Action>) -> Result<()> {
    let hints_layout = Layout::default()
      .direction(Direction::Vertical)
      .constraints(match f.area().width {
        x if x < 160 => [Constraint::Fill(1), Constraint::Length(2)],
        _ => [Constraint::Fill(1), Constraint::Length(1)],
      })
      .split(f.area());
    let root_layout = Layout::default()
      .direction(Direction::Horizontal)
      .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
      .split(hints_layout[0]);
    let right_layout = Layout::default()
      .direction(Direction::Vertical)
      .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
      .split(root_layout[1]);
    let tabs_layout = Layout::default()
      .direction(Direction::Vertical)
      .constraints([Constraint::Length(1), Constraint::Fill(1)])
      .split(right_layout[0]);

    if let Some(event) = &self.last_frame_mouse_event
      && self.popup.is_none()
      && event.kind != MouseEventKind::Moved
      && !matches!(event.kind, MouseEventKind::Down(_))
    {
      let position = Position::new(event.column, event.row);
      let menu_target = root_layout[0];
      let tabs_target = tabs_layout[0];
      let tab_content_target = tabs_layout[1];
      let data_target = right_layout[1];
      if menu_target.contains(position) {
        self.set_focus(Focus::Menu);
      } else if data_target.contains(position) {
        self.set_focus(Focus::Data);
      } else if tab_content_target.contains(position) {
        self.last_focused_tab();
      } else if tabs_target.contains(position) {
        match self.state.focus {
          Focus::Editor => {
            if matches!(event.kind, MouseEventKind::Up(_)) {
              self.set_focus(Focus::History);
            }
          },
          Focus::History => {
            if matches!(event.kind, MouseEventKind::Up(_)) {
              self.set_focus(Focus::Favorites);
            }
          },
          Focus::Favorites => {
            if matches!(event.kind, MouseEventKind::Up(_)) {
              self.set_focus(Focus::Editor);
            }
          },
          Focus::PopUp => {},
          _ => {
            self.state.focus = self.last_focused_tab;
          },
        }
        self.last_frame_mouse_event = None;
      }
    }
    let tabs = Tabs::new(vec![" 󰤏 query <alt+2>", "   history <alt+4>", "   favorites <alt+5>"])
      .highlight_style(Style::new().fg(self.state.focus.tab_color()).reversed())
      .select(self.last_focused_tab.tab_index())
      .padding(" ", "")
      .divider(" ");

    let state = &self.state;

    f.render_widget(tabs, tabs_layout[0]);
    f.render_widget(Clear, tabs_layout[1]);

    match self.last_focused_tab {
      Focus::Editor => {
        self.components.editor.draw(f, tabs_layout[1], state).unwrap();
      },
      Focus::History => {
        self.components.history.draw(f, tabs_layout[1], state).unwrap();
      },
      Focus::Favorites => {
        self.components.favorites.draw(f, tabs_layout[1], state).unwrap();
      },
      Focus::Menu | Focus::Data | Focus::PopUp => (),
    };

    self.components.menu.draw(f, root_layout[0], state).unwrap();
    self.components.data.draw(f, right_layout[1], state).unwrap();
    self.render_hints(f, hints_layout[1]);

    if let Some(popup) = &self.popup {
      self.render_popup(f, popup.as_ref());
    }
    Ok(())
  }

  fn render_hints(&self, frame: &mut Frame, area: Rect) {
    let block = Block::default().style(Style::default().fg(Color::Blue));
    let help_text = format!(
      "{}{}",
      match self.state.query_task_running {
        false => "",
        _ if self.state.focus == Focus::Editor => "[<alt + q>] abort ",
        _ if self.state.focus != Focus::PopUp => "[q] abort ",
        _ => "",
      },
      match self.state.focus {
        Focus::Menu =>
          "[R] refresh [j|↓] down [k|↑] up [l|<enter>] table list [h|󰁮 ] schema list [/] search [g] top [G] bottom",
        Focus::Editor if !self.state.query_task_running =>
          "[<alt + enter>|<f5>] execute query [<ctrl + f>|<alt + f>] save query to favorites",
        Focus::History => "[j|↓] down [k|↑] up [y] copy query [I] edit query [D] clear history",
        Focus::Favorites =>
          "[j|↓] down [k|↑] up [y] copy query [I] edit query [D] delete entry [/] search [<esc>] clear search",
        Focus::Data if !self.state.query_task_running =>
          "[P] export [j|↓] next row [k|↑] prev row [w|e] next col [b] prev col [v] select field [V] select row [y] copy [g] top [G] bottom [0] first col [$] last col",
        Focus::PopUp => "[<esc>] cancel",
        _ => "",
      }
    );
    let paragraph = Paragraph::new(Line::from(help_text).centered()).block(block).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
  }

  fn render_popup(&self, frame: &mut Frame, popup: &dyn PopUp) {
    let area = center(frame.area(), Constraint::Percentage(50), Constraint::Percentage(50));
    let block = Block::default()
      .borders(Borders::ALL)
      .border_style(Style::default().fg(Color::Yellow))
      .title(Line::from(" Confirm Action ").centered())
      .padding(Padding::uniform(1));
    let layout = Layout::default()
      .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
      .direction(Direction::Vertical)
      .split(block.inner(area));

    let popup_cta = Paragraph::new(Line::from(popup.get_cta_text(&self.state)).centered()).wrap(Wrap { trim: false });
    let popup_actions = Paragraph::new(Line::from(popup.get_actions_text(&self.state)).centered());
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(popup_cta, layout[0]);
    frame.render_widget(popup_actions, center(layout[1], Constraint::Fill(1), Constraint::Percentage(50)));
  }
}
