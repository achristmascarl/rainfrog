use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Focus {
  #[default]
  Menu,
  Editor,
  Data,
  PopUp,
}
