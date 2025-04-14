use std::marker::PhantomData;

use crossterm::event::{KeyCode, KeyEvent};
use sqlparser::ast::Statement;
use sqlx::Either;
use tokio::sync::mpsc::UnboundedSender;

use super::{PopUp, PopUpPayload};
use crate::{
  action::Action,
  database::{statement_type_string, Rows},
};

#[derive(Debug)]
pub struct ConfirmQuery {
  pending_query: String,
  statement_type: Statement,
}

impl ConfirmQuery {
  pub fn new(pending_query: String, statement_type: Statement) -> Self {
    Self { pending_query, statement_type }
  }
}

impl PopUp for ConfirmQuery {
  fn handle_key_events(
    &mut self,
    key: crossterm::event::KeyEvent,
    app_state: &mut crate::app::AppState,
  ) -> color_eyre::eyre::Result<Option<PopUpPayload>> {
    match key.code {
      KeyCode::Char('Y') => Ok(Some(PopUpPayload::ConfirmQuery(self.pending_query.to_owned()))),
      KeyCode::Char('N') | KeyCode::Esc => Ok(Some(PopUpPayload::SetDataTable(None, None))),
      _ => Ok(None),
    }
  }

  fn get_cta_text(&self, app_state: &crate::app::AppState) -> String {
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

  fn get_actions_text(&self, app_state: &crate::app::AppState) -> String {
    "[Y]es to confirm | [N]o to cancel".to_string()
  }
}
