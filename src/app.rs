use std::{borrow::Borrow, fmt::format, sync::Arc};

use color_eyre::eyre::Result;
use crossterm::event::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use futures::{task::Poll, FutureExt};
use log::log;
use ratatui::{
  layout::{Constraint, Direction, Layout, Position},
  prelude::Rect,
  style::{Color, Style, Stylize},
  text::Line,
  widgets::{Block, Borders, Clear, Padding, Paragraph, Tabs, Wrap},
  Frame,
};
use serde::{Deserialize, Serialize};
use sqlparser::{ast::Statement, keywords::DELETE};
use sqlx::{postgres::Postgres, Either, Transaction};
use tokio::{
  sync::{
    mpsc::{self},
    Mutex,
  },
  task::JoinHandle,
};

use crate::{
  action::Action,
  components::{
    data::{Data, DataComponent},
    editor::Editor,
    history::History,
    menu::{Menu, MenuComponent},
    Component,
  },
  config::Config,
  database::{self, statement_type_string, DbError, DbPool, Rows},
  focus::Focus,
  tui,
  ui::center,
};

pub enum DbTask<'a> {
  Query(tokio::task::JoinHandle<QueryResultsWithMetadata>),
  TxStart(tokio::task::JoinHandle<(QueryResultsWithMetadata, Transaction<'a, Postgres>)>),
  TxPending(Transaction<'a, Postgres>, QueryResultsWithMetadata),
  TxCommit(tokio::task::JoinHandle<QueryResultsWithMetadata>),
}

pub struct HistoryEntry {
  pub query_lines: Vec<String>,
  pub timestamp: chrono::DateTime<chrono::Local>,
}

pub struct AppState<'a> {
  pub connection_string: String,
  pub focus: Focus,
  pub query_task: Option<DbTask<'a>>,
  pub history: Vec<HistoryEntry>,
}

pub struct Components<'a> {
  pub menu: Box<dyn MenuComponent<'a>>,
  pub editor: Box<dyn Component>,
  pub history: Box<dyn Component>,
  pub data: Box<dyn DataComponent<'a>>,
}

pub struct QueryResultsWithMetadata {
  pub results: Result<Rows, DbError>,
  pub statement_type: Statement,
}

pub struct App<'a> {
  pub config: Config,
  pub tick_rate: Option<f64>,
  pub frame_rate: Option<f64>,
  pub components: Components<'static>,
  pub should_quit: bool,
  pub last_tick_key_events: Vec<KeyEvent>,
  pub last_frame_mouse_event: Option<MouseEvent>,
  pub pool: Option<DbPool>,
  pub state: AppState<'a>,
  last_focused_tab: Focus,
}

impl<'a> App<'a> {
  pub fn new(connection_string: String, tick_rate: Option<f64>, frame_rate: Option<f64>) -> Result<Self> {
    let focus = Focus::Menu;
    let menu = Menu::new();
    let editor = Editor::new();
    let history = History::new();
    let data = Data::new();
    let config = Config::new()?;
    Ok(Self {
      tick_rate,
      frame_rate,
      components: Components {
        menu: Box::new(menu),
        editor: Box::new(editor),
        history: Box::new(history),
        data: Box::new(data),
      },
      should_quit: false,
      config,
      last_tick_key_events: Vec::new(),
      last_frame_mouse_event: None,
      pool: None,
      state: AppState { connection_string, focus, query_task: None, history: vec![] },
      last_focused_tab: Focus::Editor,
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

  pub async fn run(&mut self) -> Result<()> {
    let (action_tx, mut action_rx) = mpsc::unbounded_channel();
    let connection_url = self.state.connection_string.clone();
    let pool = database::init_pool(connection_url).await?;
    log::info!("{pool:?}");
    self.pool = Some(pool);

    let mut tui = tui::Tui::new()?.tick_rate(self.tick_rate).frame_rate(self.frame_rate);
    tui.enter()?;

    self.components.menu.register_action_handler(action_tx.clone())?;
    self.components.editor.register_action_handler(action_tx.clone())?;
    self.components.history.register_action_handler(action_tx.clone())?;
    self.components.data.register_action_handler(action_tx.clone())?;

    self.components.menu.register_config_handler(self.config.clone())?;
    self.components.editor.register_config_handler(self.config.clone())?;
    self.components.history.register_config_handler(self.config.clone())?;
    self.components.data.register_config_handler(self.config.clone())?;

    let size = tui.size()?;
    self.components.menu.init(Rect { width: size.width, height: size.height, x: 0, y: 0 })?;
    self.components.editor.init(Rect { width: size.width, height: size.height, x: 0, y: 0 })?;
    self.components.history.init(Rect { width: size.width, height: size.height, x: 0, y: 0 })?;
    self.components.data.init(Rect { width: size.width, height: size.height, x: 0, y: 0 })?;

    action_tx.send(Action::LoadMenu)?;

    loop {
      match &mut self.state.query_task {
        Some(DbTask::Query(task)) => {
          if task.is_finished() {
            let results = task.await?;
            self.state.query_task = None;
            self.components.data.set_data_state(Some(results.results), Some(results.statement_type));
          }
        },
        Some(DbTask::TxStart(task)) => {
          if task.is_finished() {
            let (results, tx) = task.await?;
            match results.results {
              Ok(_) => {
                self.state.query_task = Some(DbTask::TxPending(tx, results));
                self.state.focus = Focus::PopUp;
              },
              Err(_) => {
                self.state.query_task = None;
                self.components.data.set_data_state(Some(results.results), Some(results.statement_type));
              },
            }
          }
        },
        Some(DbTask::TxCommit(task)) => {},
        _ => {},
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
              } else if self.state.focus == Focus::PopUp {
                match key.code {
                  KeyCode::Char('Y') | KeyCode::Char('N') | KeyCode::Esc => {
                    let task = self.state.query_task.take();
                    if let Some(DbTask::TxPending(tx, results)) = task {
                      let result = match key.code {
                        KeyCode::Char('Y') => tx.commit().await,
                        KeyCode::Char('N') | KeyCode::Esc => tx.rollback().await,
                        _ => panic!("inconsistent key codes"),
                      };
                      self.components.data.set_data_state(
                        match result {
                          Ok(_) => Some(Ok((vec![], None))),
                          Err(e) => Some(Err(Either::Left(e))),
                        },
                        Some(match key.code {
                          KeyCode::Char('Y') => Statement::Commit { chain: false },
                          KeyCode::Char('N') | KeyCode::Esc => Statement::Rollback { chain: false, savepoint: None },
                          _ => panic!("inconsistent key codes"),
                        }),
                      );
                    }
                    self.state.focus = Focus::Editor;
                  },
                  _ => {},
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
          if let Some(action) =
            self.components.menu.handle_events(Some(e.clone()), self.last_tick_key_events.clone(), &self.state)?
          {
            action_tx.send(action)?;
          }
          if let Some(action) =
            self.components.editor.handle_events(Some(e.clone()), self.last_tick_key_events.clone(), &self.state)?
          {
            action_tx.send(action)?;
          }
          if let Some(action) =
            self.components.history.handle_events(Some(e.clone()), self.last_tick_key_events.clone(), &self.state)?
          {
            action_tx.send(action)?;
          }
          if let Some(action) =
            self.components.data.handle_events(Some(e.clone()), self.last_tick_key_events.clone(), &self.state)?
          {
            action_tx.send(action)?;
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
              self.draw_layout(f);
            })?;
          },
          Action::Render => {
            tui.draw(|f| {
              self.draw_layout(f);
            })?;
            self.last_frame_mouse_event = None;
          },
          Action::FocusMenu => self.state.focus = Focus::Menu,
          Action::FocusEditor => {
            self.state.focus = Focus::Editor;
            self.last_focused_tab = Focus::Editor;
          },
          Action::FocusHistory => {
            self.state.focus = Focus::History;
            self.last_focused_tab = Focus::History;
          },
          Action::FocusData => self.state.focus = Focus::Data,
          Action::LoadMenu => {
            log::info!("LoadMenu");
            if let Some(pool) = &self.pool {
              let results = database::query(
                "select table_schema, table_name
                        from information_schema.tables
                        where table_schema != 'pg_catalog'
                        and table_schema != 'information_schema'
                        group by table_schema, table_name
                        order by table_schema, table_name asc"
                  .to_owned(),
                pool,
              )
              .await;
              self.components.menu.set_table_list(Some(results));
            }
          },
          Action::Query(query_lines) => {
            self.add_to_history(query_lines.clone());
            let query_string = query_lines.clone().join(" ");
            let should_use_tx = database::should_use_tx(&query_string);
            let action_tx = action_tx.clone();
            if let Some(pool) = &self.pool {
              let pool = pool.clone();
              match should_use_tx {
                Ok(true) => {
                  self.components.data.set_loading();
                  let tx = pool.begin().await?;
                  self.state.query_task = Some(DbTask::TxStart(tokio::spawn(async move {
                    let (results, tx) = database::query_with_tx(tx, query_string.clone()).await;
                    match results {
                      Ok(rows_affected) => {
                        log::info!("{:?} rows affected", rows_affected);
                        let statement_type = database::get_statement_type(query_string.clone().as_str()).unwrap();
                        (QueryResultsWithMetadata { results: Ok((vec![], Some(rows_affected))), statement_type }, tx)
                      },
                      Err(e) => {
                        log::error!("{e:?}");
                        let statement_type = database::get_statement_type(&query_string).unwrap();
                        (QueryResultsWithMetadata { results: Err(e), statement_type }, tx)
                      },
                    }
                  })));
                },
                Ok(false) => {
                  self.components.data.set_loading();
                  self.state.query_task = Some(DbTask::Query(tokio::spawn(async move {
                    let results = database::query(query_string.clone(), &pool).await;
                    match &results {
                      Ok(rows) => {
                        log::info!("{:?} rows, {:?} affected", rows.0.len(), rows.1);
                      },
                      Err(e) => {
                        log::error!("{e:?}");
                      },
                    };
                    let statement_type = database::get_statement_type(&query_string).unwrap();

                    QueryResultsWithMetadata { results, statement_type }
                  })));
                },
                Err(e) => self.components.data.set_data_state(Some(Err(e)), None),
              }
            } else {
              log::error!("No connection pool");
              self.components.data.set_data_state(Some(Err(DbError::Left(sqlx::Error::PoolTimedOut))), None)
            }
          },
          Action::AbortQuery => {
            match &self.state.query_task {
              Some(DbTask::Query(task)) => {
                task.abort();
                self.state.query_task = None;
                self.components.data.set_cancelled();
              },
              Some(DbTask::TxStart(task)) => {
                task.abort();
                self.state.query_task = None;
                self.components.data.set_cancelled();
              },
              _ => {},
            }
          },
          Action::ClearHistory => {
            self.clear_history();
          },
          _ => {},
        }
        if !action_consumed {
          if let Some(action) = self.components.menu.update(action.clone(), &self.state)? {
            action_tx.send(action)?;
          }
          if let Some(action) = self.components.editor.update(action.clone(), &self.state)? {
            action_tx.send(action)?;
          }
          if let Some(action) = self.components.history.update(action.clone(), &self.state)? {
            action_tx.send(action)?;
          }
          if let Some(action) = self.components.data.update(action.clone(), &self.state)? {
            action_tx.send(action)?;
          }
        }
      }
      if self.last_frame_mouse_event.is_some() {
        tui.draw(|f| {
          self.draw_layout(f);
        })?;
      }
      if self.should_quit {
        tui.stop()?;
        break;
      }
    }
    tui.exit()?;
    Ok(())
  }

  fn draw_layout(&mut self, f: &mut Frame) {
    let hints_layout = Layout::default()
      .direction(Direction::Vertical)
      .constraints(match f.area().width {
        x if x < 135 => [Constraint::Fill(1), Constraint::Length(2)],
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
    let tabs = Tabs::new(vec![" 󰤏 query <alt+2>", "   history <alt+3>"])
      .highlight_style(
        Style::new()
          .fg(if self.state.focus == Focus::Editor || self.state.focus == Focus::History {
            Color::Green
          } else {
            Color::default()
          })
          .reversed(),
      )
      .select(if self.last_focused_tab == Focus::Editor { 0 } else { 1 })
      .padding(" ", "")
      .divider(" ");

    if let Some(event) = &self.last_frame_mouse_event {
      if !matches!(self.state.query_task, Some(DbTask::TxPending(_, _))) && event.kind != MouseEventKind::Moved {
        let position = Position::new(event.column, event.row);
        let menu_target = root_layout[0];
        let editor_target = right_layout[0];
        let data_target = right_layout[1];
        if menu_target.contains(position) {
          self.state.focus = Focus::Menu;
        } else if editor_target.contains(position) {
          self.state.focus = Focus::Editor;
        } else if data_target.contains(position) {
          self.state.focus = Focus::Data;
        }
      }
    }

    let state = &self.state;

    f.render_widget(tabs, tabs_layout[0]);
    if self.last_focused_tab == Focus::Editor {
      self.components.editor.draw(f, tabs_layout[1], state).unwrap();
    } else {
      self.components.history.draw(f, tabs_layout[1], state).unwrap();
    }
    self.components.menu.draw(f, root_layout[0], state).unwrap();
    self.components.data.draw(f, right_layout[1], state).unwrap();
    self.render_hints(f, hints_layout[1]);

    if let Some(DbTask::TxPending(tx, results)) = &self.state.query_task {
      self.render_popup(f, results);
    }
  }

  fn render_hints(&self, frame: &mut Frame, area: Rect) {
    let block = Block::default().style(Style::default().fg(Color::Blue));
    let help_text = format!(
        "{}{}",
        match self.state.query_task {
            None => "",
            _ if self.state.focus != Focus::PopUp => "[q] abort ",
            _ => ""
        },
        match self.state.focus {
            Focus::Menu  => "[R] refresh [j|↓] down [k|↑] up [l|<enter>] table list [h|󰁮 ] schema list [/] search [g] top [G] bottom",
            Focus::Editor if self.state.query_task.is_none() => "[<alt + enter>|<f5>] execute query",
            Focus::History => "[j|↓] down [k|↑] up [y] copy query [I] edit query [D] clear history",
            Focus::Data if self.state.query_task.is_none() => "[j|↓] next row [k|↑] prev row [w|e] next col [b] prev col [v] select field [V] select row [g] top [G] bottom [0] first col [$] last col",
            Focus::PopUp => "[<esc>] cancel",
            _ => "",
        }
    );
    let paragraph = Paragraph::new(Line::from(help_text).centered()).block(block).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
  }

  fn render_popup(&self, frame: &mut Frame, results: &QueryResultsWithMetadata) {
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

    let rows_affected = match results.results {
      Ok((_, Some(n))) => n,
      _ => 0,
    };
    let cta = match results.statement_type {
      Statement::Delete(_) | Statement::Insert(_) | Statement::Update { .. } => {
        format!(
          "Are you sure you want to {} {} rows?",
          statement_type_string(&results.statement_type).to_uppercase(),
          rows_affected
        )
      },
      _ => {
        format!(
          "Are you sure you want to use a {} statement?",
          statement_type_string(&results.statement_type).to_uppercase()
        )
      },
    };
    let popup_cta = Paragraph::new(Line::from(cta).centered());
    let popup_actions = Paragraph::new(Line::from("[Y]es to confirm | [N]o to cancel").centered());
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(popup_cta, center(layout[0], Constraint::Fill(1), Constraint::Percentage(50)));
    frame.render_widget(popup_actions, center(layout[1], Constraint::Fill(1), Constraint::Percentage(50)));
  }
}
