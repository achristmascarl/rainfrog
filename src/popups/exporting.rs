use std::marker::PhantomData;

use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyEvent};
use sqlparser::ast::Statement;
use sqlx::Either;
use tokio::sync::mpsc::UnboundedSender;

use super::{PopUp, PopUpPayload};
use crate::{
  action::Action,
  app::DbTask,
  database::{statement_type_string, Rows},
};

#[derive(Debug, Default)]
pub struct Exporting<DB: sqlx::Database> {
  phantom: PhantomData<DB>,
}

impl<DB: sqlx::Database> Exporting<DB> {
  pub fn new() -> Self {
    Self { phantom: PhantomData }
  }
}

#[async_trait(?Send)]
impl<DB: sqlx::Database> PopUp<DB> for Exporting<DB> {
  async fn handle_key_events(
    &mut self,
    key: crossterm::event::KeyEvent,
    app_state: &mut crate::app::AppState<'_, DB>,
  ) -> color_eyre::eyre::Result<Option<PopUpPayload>> {
    Ok(None)
  }

  fn get_cta_text(&self, app_state: &crate::app::AppState<'_, DB>) -> String {
    "Exporting...".to_string()
  }

  fn get_actions_text(&self, app_state: &crate::app::AppState<'_, DB>) -> String {
    "".to_string()
  }
}
