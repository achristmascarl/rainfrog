use std::marker::PhantomData;

use async_trait::async_trait;
use crossterm::event::{Event, KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use sqlparser::ast::Statement;
use sqlx::Either;
use tokio::sync::mpsc::UnboundedSender;

use super::{PopUp, PopUpPayload};
use crate::{
  action::Action,
  app::DbTask,
  database::{statement_type_string, Rows},
  focus::Focus,
};

#[derive(Debug)]
pub struct ConfirmQuery<DB: sqlx::Database> {
  pending_query: String,
  statement_type: Statement,
  phantom: PhantomData<DB>,
}

impl<DB: sqlx::Database> ConfirmQuery<DB> {
  pub fn new(pending_query: String, statement_type: Statement) -> Self {
    Self { pending_query, statement_type, phantom: PhantomData }
  }
}

#[async_trait(?Send)]
impl<DB: sqlx::Database> PopUp<DB> for ConfirmQuery<DB> {
  async fn handle_key_events(
    &self,
    key: crossterm::event::KeyEvent,
    app_state: &mut crate::app::AppState<'_, DB>,
  ) -> color_eyre::eyre::Result<Option<PopUpPayload>> {
    match key.code {
      KeyCode::Char('Y') => Ok(Some(PopUpPayload::ConfirmQuery(self.pending_query.clone()))),
      KeyCode::Char('N') | KeyCode::Esc => Ok(Some(PopUpPayload::SetDataTable(None, None))),
      _ => Ok(None),
    }
  }

  fn get_cta_text(&self, app_state: &crate::app::AppState<'_, DB>) -> String {
    match self.statement_type.clone() {
      Statement::Explain { statement, .. } => {
        format!(
          "Are you sure you want to run an EXPLAIN ANALYZE that will run a {} statement?",
          statement_type_string(&statement).to_uppercase(),
        )
      },
      _ => {
        format!(
          "Are you sure you want to use a {} statement?",
          statement_type_string(&self.statement_type).to_uppercase()
        )
      },
    }
  }

  fn get_actions_text(&self, app_state: &crate::app::AppState<'_, DB>) -> String {
    "[Y]es to confirm | [N]o to cancel".to_string()
  }
}
