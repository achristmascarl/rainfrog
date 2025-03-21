use ratatui::style::Color;
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Focus {
  #[default]
  Menu,
  Editor,
  History,
  Data,
  PopUp,
  Favorites,
}

impl Focus {
  pub fn color(&self) -> Color {
    match self {
      Focus::Editor | Focus::History | Focus::Favorites => Color::Green,
      Focus::Menu | Focus::Data | Focus::PopUp => Color::default(),
    }
  }

  pub fn tab_index(&self) -> usize {
    match self {
      Focus::Editor => 0,
      Focus::History => 1,
      Focus::Favorites | Focus::Menu | Focus::Data | Focus::PopUp => 2,
    }
  }
}
