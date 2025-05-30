use super::{PopUp, PopUpPayload};

#[derive(Debug, Default)]
pub struct Exporting {}

impl Exporting {
  pub fn new() -> Self {
    Self {}
  }
}

impl PopUp for Exporting {
  fn handle_key_events(
    &mut self,
    key: crossterm::event::KeyEvent,
    app_state: &mut crate::app::AppState,
  ) -> color_eyre::eyre::Result<Option<PopUpPayload>> {
    Ok(None)
  }

  fn get_cta_text(&self, app_state: &crate::app::AppState) -> String {
    "Exporting...".to_string()
  }

  fn get_actions_text(&self, app_state: &crate::app::AppState) -> String {
    "".to_string()
  }
}
