use std::{borrow::Borrow, fmt::format, sync::Arc};

use color_eyre::eyre::Result;
use crossterm::event::KeyEvent;
use futures::{task::Poll, FutureExt};
use log::log;
use ratatui::{
  layout::{Constraint, Direction, Layout},
  prelude::Rect,
  widgets::{Block, Borders, Paragraph},
  Frame,
};
use serde::{Deserialize, Serialize};
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
  database::{self, DbError, DbPool, Rows},
  focus::Focus,
  tui,
};

pub struct AppState {
  pub connection_string: String,
  pub focus: Focus,
  pub query_task: Option<tokio::task::JoinHandle<Result<Rows, DbError>>>,
}

pub struct Components<'a> {
  pub menu: Box<dyn MenuComponent<'a>>,
  pub editor: Box<dyn Component>,
  pub data: Box<dyn DataComponent<'a>>,
}

// // TODO: see if the into_super_ref_mut fn can be fixed
//
// impl Components {
//   pub fn to_array(&mut self) -> [&mut Box<dyn Component>; 3] {
//     [&mut self.menu, &mut self.editor, self.data.into_super_ref_mut()]
//   }
// }

pub struct App {
  pub config: Config,
  pub tick_rate: Option<f64>,
  pub frame_rate: Option<f64>,
  pub components: Components<'static>,
  pub should_quit: bool,
  pub last_tick_key_events: Vec<KeyEvent>,
  pub pool: Option<DbPool>,
  pub state: AppState,
}

impl App {
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
      if let Some(query_task) = &mut self.state.query_task {
        if query_task.is_finished() {
          let results = query_task.await?;
          self.state.query_task = None;
          self.components.data.set_data_state(Some(results));
        }
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
            };
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
            let action_tx = action_tx.clone();
            if let Some(pool) = &self.pool {
              let pool = pool.clone();
              self.components.data.set_loading();
              self.state.query_task = Some(tokio::spawn(async move {
                log::info!("Query: {}", query);
                let results = database::query(query, &pool).await;
                match &results {
                  Ok(rows) => {
                    log::info!("{:?} rows, {:?} affected", rows.0.len(), rows.1);
                  },
                  Err(e) => {
                    log::error!("{e:?}");
                  },
                };
                results
              }));
            }
          },
          Action::AbortQuery => {
            if let Some(query_task) = &self.state.query_task {
              query_task.abort();
              self.state.query_task = None;
              self.components.data.set_cancelled();
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
  }
}
