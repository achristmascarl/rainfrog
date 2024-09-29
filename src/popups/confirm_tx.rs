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
pub struct ConfirmTx<DB: sqlx::Database> {
  command_tx: UnboundedSender<Action>,
  phantom: PhantomData<DB>,
}

#[async_trait(?Send)]
impl<DB: sqlx::Database> PopUp<DB> for ConfirmTx<DB> {
  fn new(tx: UnboundedSender<Action>) -> Self {
    Self { command_tx: tx.clone(), phantom: PhantomData }
  }

  async fn handle_key_events(
    &self,
    key: crossterm::event::KeyEvent,
    app_state: &mut crate::app::AppState<'_, DB>,
  ) -> color_eyre::eyre::Result<Option<PopUpPayload>> {
    match key.code {
      KeyCode::Char('Y') | KeyCode::Char('N') | KeyCode::Esc => {
        let task = app_state.query_task.take();
        if let Some(DbTask::TxPending(tx, results)) = task {
          let mut rolled_back = false;
          let result = match key.code {
            KeyCode::Char('Y') => tx.commit().await,
            KeyCode::Char('N') | KeyCode::Esc => {
              rolled_back = true;
              tx.rollback().await
            },
            _ => panic!("inconsistent key codes"),
          };
          Ok(Some(PopUpPayload::SetDataTable(
            match result {
              Ok(_) => {
                match results.statement_type {
                  Statement::Explain { .. } if results.results.is_ok() && !rolled_back => {
                    Some(Ok(results.results.unwrap()))
                  },
                  _ => Some(Ok(Rows { headers: vec![], rows: vec![], rows_affected: None })),
                }
              },
              Err(e) => Some(Err(Either::Left(e))),
            },
            Some(match rolled_back {
              false => {
                match results.statement_type {
                  Statement::Explain { .. } => results.statement_type,
                  _ => Statement::Commit { chain: false },
                }
              },
              true => Statement::Rollback { chain: false, savepoint: None },
            }),
          )))
        } else {
          Ok(None)
        }
      },
      _ => Ok(None),
    }
  }

  fn get_cta_text(&self, app_state: &crate::app::AppState<'_, DB>) -> String {
    if let Some(DbTask::TxPending(tx, results)) = &app_state.query_task {
      let rows_affected = match results.results {
        Ok(Rows { rows_affected: Some(n), .. }) => n,
        _ => 0,
      };
      match results.statement_type.clone() {
        Statement::Delete(_) | Statement::Insert(_) | Statement::Update { .. } => {
          format!(
            "Are you sure you want to {} {} rows?",
            statement_type_string(&results.statement_type).to_uppercase(),
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
            statement_type_string(&results.statement_type).to_uppercase()
          )
        },
      }
    } else {
      "No transaction pending".to_string()
    }
  }

  fn get_actions_text(&self, app_state: &crate::app::AppState<'_, DB>) -> String {
    "[Y]es to confirm | [N]o to cancel".to_string()
  }
}
