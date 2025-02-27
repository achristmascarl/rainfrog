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
pub enum ExportFormat {
  CSV,
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
  Query(Vec<String>, bool),                 // (query_lines, execution_confirmed)
  MenuPreview(MenuPreview, String, String), // (preview, schema, table)
  HistoryToEditor(Vec<String>),
  ClearHistory,
  AbortQuery,
  FocusMenu,
  FocusEditor,
  FocusHistory,
  FocusData,
  CycleFocusForwards,
  CycleFocusBackwards,
  LoadMenu,
  CopyData(String),
  RequestExportData(i64),
  ExportData(ExportFormat),
  ExportDataFinished,
}
