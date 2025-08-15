use crossterm::event::KeyCode;

use super::{PopUp, PopUpPayload};

#[derive(Debug)]
pub struct ConfirmBypass {
  pending_query: String,
}

impl ConfirmBypass {
  pub fn new(pending_query: String) -> Self {
    Self { pending_query }
  }
}

impl PopUp for ConfirmBypass {
  fn handle_key_events(
    &mut self,
    key: crossterm::event::KeyEvent,
    app_state: &mut crate::app::AppState,
  ) -> color_eyre::eyre::Result<Option<PopUpPayload>> {
    match key.code {
      KeyCode::Char('Y') => Ok(Some(PopUpPayload::ConfirmBypass(self.pending_query.to_owned()))),
      KeyCode::Char('N') | KeyCode::Esc => Ok(Some(PopUpPayload::SetDataTable(None, None))),
      _ => Ok(None),
    }
  }

  fn get_cta_text(&self, app_state: &crate::app::AppState) -> String {
    "Are you sure you want to bypass the query parser? The query will not be wrapped in a transaction, so it cannot be undone.".to_string()
  }

  fn get_actions_text(&self, app_state: &crate::app::AppState) -> String {
    "[Y]es to confirm | [N]o to cancel".to_string()
  }
}
