use std::marker::PhantomData;

use async_trait::async_trait;
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
pub struct ConfirmExport {
  row_count: i64,
}

impl ConfirmExport {
  pub fn new(row_count: i64) -> Self {
    Self { row_count }
  }
}

#[async_trait(?Send)]
impl PopUp for ConfirmExport {
  async fn handle_key_events(
    &mut self,
    key: crossterm::event::KeyEvent,
    app_state: &mut crate::app::AppState,
  ) -> color_eyre::eyre::Result<Option<PopUpPayload>> {
    match key.code {
      KeyCode::Char('Y') => Ok(Some(PopUpPayload::ConfirmExport(true))),
      KeyCode::Char('N') | KeyCode::Esc => Ok(Some(PopUpPayload::ConfirmExport(false))),
      _ => Ok(None),
    }
  }

  fn get_cta_text(&self, app_state: &crate::app::AppState) -> String {
    format!(
      "Are you sure you want to export {} rows? Exporting too many rows may cause the app to hang.",
      self.row_count,
    )
  }

  fn get_actions_text(&self, app_state: &crate::app::AppState) -> String {
    "[Y]es to confirm | [N]o to cancel".to_string()
  }
}
