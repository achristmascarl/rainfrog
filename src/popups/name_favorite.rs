use crossterm::event::KeyCode;

use super::{PopUp, PopUpPayload};

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

  fn push_char(&mut self, c: char) {
    // Preserve the existing favorite-name sanitization for typed and pasted input.
    if c.is_whitespace()
      || c.is_ascii_whitespace()
      || (c.is_ascii_punctuation() && c != '_' && c != '-')
    {
      return;
    }
    self.name.push(c);
  }

  fn push_input(&mut self, input: &str) {
    for c in input.chars() {
      self.push_char(c);
    }
  }
}

impl PopUp for NameFavorite {
  fn handle_key_events(
    &mut self,
    key: crossterm::event::KeyEvent,
    app_state: &mut crate::app::AppState,
  ) -> color_eyre::eyre::Result<Option<PopUpPayload>> {
    match key.code {
      KeyCode::Char(c) => {
        self.push_char(c);
        Ok(None)
      },
      KeyCode::Enter => {
        let favorite_name = self.name.trim();
        if !favorite_name.is_empty() {
          return Ok(Some(PopUpPayload::NamedFavorite(
            favorite_name.to_string(),
            self.query_lines.clone(),
          )));
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

  fn handle_paste_events(
    &mut self,
    text: &str,
    app_state: &mut crate::app::AppState,
  ) -> color_eyre::eyre::Result<()> {
    self.push_input(text);
    Ok(())
  }

  fn get_cta_text(&self, app_state: &crate::app::AppState) -> String {
    "Input a name for the favorite and then press [Enter]; press [Esc] to cancel. No spaces or special characters allowed.".to_string()
  }

  fn get_actions_text(&self, app_state: &crate::app::AppState) -> String {
    format!(
      "{}.sql{}",
      self.name,
      if self.existing_names.iter().any(|n| n.as_str().trim() == self.name.as_str().trim()) {
        " (WARNING! a favorite with this name already exists, saving now will overwrite it.)"
      } else {
        ""
      }
    )
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{components::app_state_with_focus, focus::Focus};

  #[test]
  fn paste_uses_the_same_name_sanitization_as_typed_input() {
    let mut favorite = NameFavorite::new(Vec::new(), vec!["select 1".to_string()]);
    let mut app_state = app_state_with_focus(Focus::PopUp);

    favorite.handle_paste_events("my favorite!?_v2-東京\n", &mut app_state).unwrap();

    assert_eq!(favorite.name, "myfavorite_v2-東京");
  }
}
