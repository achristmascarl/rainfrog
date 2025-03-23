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

#[derive(Debug)]
pub struct NameFavorite<DB: sqlx::Database> {
  name: String,
  query_lines: Vec<String>,
  phantom: PhantomData<DB>,
}

impl<DB: sqlx::Database> NameFavorite<DB> {
  pub fn new(query_lines: Vec<String>) -> Self {
    Self { name: "".to_string(), query_lines, phantom: PhantomData }
  }
}

#[async_trait(?Send)]
impl<DB: sqlx::Database> PopUp<DB> for NameFavorite<DB> {
  async fn handle_key_events(
    &mut self,
    key: crossterm::event::KeyEvent,
    app_state: &mut crate::app::AppState<'_, DB>,
  ) -> color_eyre::eyre::Result<Option<PopUpPayload>> {
    match key.code {
      KeyCode::Char(c) => {
        self.name.push(c);
        Ok(None)
      },
      KeyCode::Enter => Ok(None),
      KeyCode::Esc => Ok(Some(PopUpPayload::Cancel)),
      KeyCode::Backspace => {
        if !self.name.is_empty() {
          self.name.pop();
        }
        Ok(None)
      },
      _ => Ok(None),
    }
  }

  fn get_cta_text(&self, app_state: &crate::app::AppState<'_, DB>) -> String {
    "Input a name for the favorite and then press [Enter]. No spaces or special characters allowed.".to_string()
  }

  fn get_actions_text(&self, app_state: &crate::app::AppState<'_, DB>) -> String {
    format!("{}.sql", self.name)
  }
}
