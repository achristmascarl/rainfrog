use crossterm::event::KeyCode;

use super::{PopUp, PopUpPayload};

#[derive(Debug)]
pub struct ConfirmExport {
  row_count: i64,
}

impl ConfirmExport {
  pub fn new(row_count: i64) -> Self {
    Self { row_count }
  }
}

impl PopUp for ConfirmExport {
  fn handle_key_events(
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
