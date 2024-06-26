use std::{fmt, string::ToString};

use serde::{
  de::{self, Deserializer, Visitor},
  Deserialize, Serialize,
};
use strum::Display;

use crate::focus::Focus;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Display, Deserialize)]
pub enum Action {
  Tick,
  Render,
  Resize(u16, u16),
  Resume,
  Quit,
  Refresh,
  Error(String),
  Help,
  Query(String),
  FocusMenu,
  FocusEditor,
  FocusData,
}
