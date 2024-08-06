use std::{borrow::Borrow, fmt::format, sync::Arc};

use color_eyre::eyre::Result;
use crossterm::event::{Event, KeyCode, KeyEvent};
use futures::{task::Poll, FutureExt};
use log::log;
use ratatui::{
  layout::{Constraint, Direction, Layout},
  prelude::Rect,
  style::{Color, Style},
  text::Line,
  widgets::{Block, Borders, Clear, Padding, Paragraph},
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

pub struct AppState<'a> {
  pub connection_string: String,
  pub focus: Focus,
  pub query_task: Option<DbTask<'a>>,
}

pub struct Components<'a> {
  pub menu: Box<dyn MenuComponent<'a>>,
  pub editor: Box<dyn Component>,
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
  pub pool: Option<DbPool>,
  pub state: AppState<'a>,
}

impl<'a> App<'a> {
  pub fn new(connection_string: String, tick_rate: Option<f64>, frame_rate: Option<f64>) -> Result<Self> {
    let focus = Focus::Menu;
    let menu = Menu::new();
    let editor = Editor::new();
    let data = Data::new();
    let config = Config::new()?;
    Ok(Self {
      tick_rate,
      frame_rate,
      components: Components { menu: Box::new(menu), editor: Box::new(editor), data: Box::new(data) },
      should_quit: false,
      config,
      last_tick_key_events: Vec::new(),
      pool: None,
      state: AppState { connection_string, focus, query_task: None },
    })
  }

  pub async fn run(&mut self) -> Result<()> {
    let (action_tx, mut action_rx) = mpsc::unbounded_channel();
    let connection_url = self.state.connection_string.clone();
    let pool = database::init_pool(connection_url).await?;
    log::info!("{pool:?}");
    self.pool = Some(pool);

    let mut tui = tui::Tui::new()?.tick_rate(self.tick_rate).frame_rate(self.frame_rate);
    // tui.mouse(true);
    tui.enter()?;

    self.components.menu.register_action_handler(action_tx.clone())?;
    self.components.editor.register_action_handler(action_tx.clone())?;
    self.components.data.register_action_handler(action_tx.clone())?;

    self.components.menu.register_config_handler(self.config.clone())?;
    self.components.editor.register_config_handler(self.config.clone())?;
    self.components.data.register_config_handler(self.config.clone())?;

    self.components.menu.init(tui.size()?)?;
    self.components.editor.init(tui.size()?)?;
    self.components.data.init(tui.size()?)?;

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
          },
          Action::FocusMenu => {
            log::info!("FocusMenu");
            self.state.focus = Focus::Menu;
          },
          Action::FocusEditor => {
            log::info!("FocusEditor");
            self.state.focus = Focus::Editor;
          },
          Action::FocusData => {
            log::info!("FocusData");
            self.state.focus = Focus::Data;
          },
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
          Action::Query(query) => {
            let query = query.to_owned();
            let should_use_tx = database::should_use_tx(&query);
            let action_tx = action_tx.clone();
            if let Some(pool) = &self.pool {
              let pool = pool.clone();
              match should_use_tx {
                Ok(true) => {
                  self.components.data.set_loading();
                  let tx = pool.begin().await?;
                  self.state.query_task = Some(DbTask::TxStart(tokio::spawn(async move {
                    log::info!("Tx Query: {}", query);
                    let (results, tx) = database::query_with_tx(tx, query.clone()).await;
                    match results {
                      Ok(rows_affected) => {
                        log::info!("{:?} rows affected", rows_affected);
                        let statement_type = database::get_statement_type(query.clone().as_str()).unwrap();
                        (QueryResultsWithMetadata { results: Ok((vec![], Some(rows_affected))), statement_type }, tx)
                      },
                      Err(e) => {
                        log::error!("{e:?}");
                        let statement_type = database::get_statement_type(&query).unwrap();
                        (QueryResultsWithMetadata { results: Err(e), statement_type }, tx)
                      },
                    }
                  })));
                },
                Ok(false) => {
                  self.components.data.set_loading();
                  self.state.query_task = Some(DbTask::Query(tokio::spawn(async move {
                    log::info!("Query: {}", query);
                    let results = database::query(query.clone(), &pool).await;
                    match &results {
                      Ok(rows) => {
                        log::info!("{:?} rows, {:?} affected", rows.0.len(), rows.1);
                      },
                      Err(e) => {
                        log::error!("{e:?}");
                      },
                    };
                    let statement_type = database::get_statement_type(&query).unwrap();

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
          _ => {},
        }
        if !action_consumed {
          if let Some(action) = self.components.menu.update(action.clone(), &self.state)? {
            action_tx.send(action)?;
          }
          if let Some(action) = self.components.editor.update(action.clone(), &self.state)? {
            action_tx.send(action)?;
          }
          if let Some(action) = self.components.data.update(action.clone(), &self.state)? {
            action_tx.send(action)?;
          }
        }
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
    let root_layout = Layout::default()
      .direction(Direction::Horizontal)
      .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
      .split(f.size());
    let right_layout = Layout::default()
      .direction(Direction::Vertical)
      .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
      .split(root_layout[1]);
    let state = &self.state;

    self.components.menu.draw(f, root_layout[0], state).unwrap();
    self.components.editor.draw(f, right_layout[0], state).unwrap();
    self.components.data.draw(f, right_layout[1], state).unwrap();

    if let Some(DbTask::TxPending(tx, results)) = &self.state.query_task {
      self.render_popup(f, results);
    }
  }

  fn render_popup(&self, frame: &mut Frame, results: &QueryResultsWithMetadata) {
    let area = center(frame.size(), Constraint::Percentage(60), Constraint::Percentage(60));
    let block = Block::default()
      .borders(Borders::ALL)
      .border_style(Style::default().fg(Color::Green))
      .title(Line::from("Confirm Action").centered())
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
    let popup_actions = Paragraph::new(Line::from("(Y)es to confirm | (N)o to cancel").centered());
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(popup_cta, center(layout[0], Constraint::Fill(1), Constraint::Percentage(50)));
    frame.render_widget(popup_actions, center(layout[1], Constraint::Fill(1), Constraint::Percentage(50)));
  }
}
