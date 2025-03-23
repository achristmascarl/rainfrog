use async_trait::async_trait;
use color_eyre::eyre::Result;
use crossterm::event::KeyEvent;
use sqlparser::ast::Statement;

use crate::{
  app::AppState,
  database::{DbError, Rows},
};

pub mod confirm_export;
pub mod confirm_query;
pub mod confirm_tx;
pub mod exporting;
pub mod name_favorite;

// since popups are meant to overlay the entire app and capture
// all input, we have a payload representing when a popup is exited
// and some action by the main thread is desired. easier than making
// it work with Actions for now.
#[allow(clippy::large_enum_variant)]
pub enum PopUpPayload {
  Cancel, // does nothing and closes the popup
  SetDataTable(Option<Result<Rows, DbError>>, Option<Statement>),
  ConfirmQuery(String),
  ConfirmExport(bool),
  NamedFavorite(String, Vec<String>),
}

#[async_trait(?Send)]
pub trait PopUp<DB: sqlx::Database> {
  #[allow(unused_variables)]
  async fn handle_key_events(
    &mut self,
    key: KeyEvent,
    app_state: &mut AppState<'_, DB>,
  ) -> Result<Option<PopUpPayload>>;

  #[allow(unused_variables)]
  fn get_cta_text(&self, app_state: &AppState<'_, DB>) -> String {
    "".to_string()
  }

  #[allow(unused_variables)]
  fn get_actions_text(&self, app_state: &AppState<'_, DB>) -> String {
    "".to_string()
  }
}
