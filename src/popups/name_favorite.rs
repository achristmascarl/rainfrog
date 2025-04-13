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
pub struct NameFavorite {
  name: String,
  existing_names: Vec<String>,
  query_lines: Vec<String>,
}

impl NameFavorite {
  pub fn new(existing_names: Vec<String>, query_lines: Vec<String>) -> Self {
    Self { name: "".to_string(), existing_names, query_lines }
  }
}

#[async_trait(?Send)]
impl PopUp for NameFavorite {
  async fn handle_key_events(
    &mut self,
    key: crossterm::event::KeyEvent,
    app_state: &mut crate::app::AppState,
  ) -> color_eyre::eyre::Result<Option<PopUpPayload>> {
    match key.code {
      KeyCode::Char(c) => {
        // ignore invalid characters
        if c.is_ascii_whitespace() || (c.is_ascii_punctuation() && c != '_' && c != '-') {
          return Ok(None);
        }
        self.name.push(c);
        Ok(None)
      },
      KeyCode::Enter => {
        let favorite_name = self.name.trim();
        if !favorite_name.is_empty() {
          return Ok(Some(PopUpPayload::NamedFavorite(favorite_name.to_string(), self.query_lines.clone())));
        }
        Ok(None)
      },
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

  fn get_cta_text(&self, app_state: &crate::app::AppState) -> String {
    "Input a name for the favorite and then press [Enter]; press [Esc] to cancel. No spaces or special characters allowed.".to_string()
  }

  fn get_actions_text(&self, app_state: &crate::app::AppState) -> String {
    format!(
      "{}.sql{}",
      self.name,
      if self.existing_names.iter().any(|n| n.as_str() == self.name.as_str()) {
        " (WARNING! a favorite with this name already exists, saving now will overwrite it.)"
      } else {
        ""
      }
    )
  }
}
