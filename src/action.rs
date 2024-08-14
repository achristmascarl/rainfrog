use std::{fmt, string::ToString};

use serde::{
  de::{self, Deserializer, Visitor},
  Deserialize, Serialize,
};
use strum::Display;

use crate::{
  database::{DbError, Rows},
  focus::Focus,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Display, Deserialize)]
pub enum MenuPreview {
  Rows,
  Columns,
  Constraints,
  Indexes,
  Policies,
}

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
  SubmitEditorQuery,
  Query(String),
  MenuPreview(MenuPreview, String, String), // (preview, schema, table)
  AbortQuery,
  FocusMenu,
  FocusEditor,
  FocusHistory,
  FocusData,
  LoadMenu,
  CopyData(String),
}
