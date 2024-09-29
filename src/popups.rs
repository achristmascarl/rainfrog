use async_trait::async_trait;
use color_eyre::eyre::Result;
use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::Rect;
use sqlparser::ast::Statement;
use tokio::sync::mpsc::UnboundedSender;

use crate::{
  action::Action,
  app::AppState,
  config::Config,
  database::{DbError, Rows},
  tui::{Event, Frame},
};

pub mod confirm_query;
pub mod confirm_tx;

// since popups are meant to overlay the entire app and capture
// all input, we have a payload representing when a popup is exited
// and some action by the main thread is desired. easier than making
// it work with Actions for now.
pub enum PopUpPayload {
  SetDataTable(Option<Result<Rows, DbError>>, Option<Statement>),
}

#[async_trait(?Send)]
pub trait PopUp<DB: sqlx::Database> {
  fn new(tx: UnboundedSender<Action>) -> Self
  where
    Self: Sized;

  #[allow(unused_variables)]
  async fn handle_key_events(&self, key: KeyEvent, app_state: &mut AppState<'_, DB>) -> Result<Option<PopUpPayload>>;

  #[allow(unused_variables)]
  fn get_cta_text(&self, app_state: &AppState<'_, DB>) -> String {
    "".to_string()
  }

  #[allow(unused_variables)]
  fn get_actions_text(&self, app_state: &AppState<'_, DB>) -> String {
    "".to_string()
  }
}
