use crossterm::event::KeyCode;
use sqlparser::ast::Statement;

use super::{PopUp, PopUpPayload};
use crate::database::statement_type_string;

#[derive(Debug)]
pub struct ConfirmTx {
  rows_affected: Option<u64>,
  statement_type: Statement,
}

impl ConfirmTx {
  pub fn new(rows_affected: Option<u64>, statement_type: Statement) -> Self {
    Self { rows_affected, statement_type }
  }
}

impl PopUp for ConfirmTx {
  fn handle_key_events(
    &mut self,
    key: crossterm::event::KeyEvent,
    app_state: &mut crate::app::AppState,
  ) -> color_eyre::eyre::Result<Option<PopUpPayload>> {
    match key.code {
      KeyCode::Char('Y') => Ok(Some(PopUpPayload::CommitTx)),
      KeyCode::Char('N') | KeyCode::Esc => Ok(Some(PopUpPayload::RollbackTx)),
      _ => Ok(None),
    }
  }

  fn get_cta_text(&self, app_state: &crate::app::AppState) -> String {
    let rows_affected = self.rows_affected.unwrap_or_default();
    match self.statement_type.clone() {
      Statement::Delete(_) | Statement::Insert(_) | Statement::Update { .. } => {
        format!(
          "Are you sure you want to {} {} rows?",
          statement_type_string(&self.statement_type).to_uppercase(),
          rows_affected
        )
      },
      Statement::Explain { statement, .. } => {
        format!(
          "Are you sure you want to run an EXPLAIN ANALYZE that will {} rows?",
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
