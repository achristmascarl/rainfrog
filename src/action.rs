use serde::{Deserialize, Serialize};
use strum::Display;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Display, Deserialize)]
pub enum MenuPreview {
  Rows,
  Columns,
  Constraints,
  Indexes,
  Policies,
  Definition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MenuItemKind {
  Table,
  View { materialized: bool },
  Function,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuTarget {
  pub schema: String,
  pub name: String,
  pub kind: MenuItemKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Display, Deserialize)]
pub enum ExportFormat {
  CSV,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Display, Deserialize)]
pub enum Action {
  // An explicitly disabled keybinding. The empty serde name lets it act as a
  // tombstone while user bindings are merged with the defaults.
  #[serde(rename = "")]
  NoOp,
  Tick,
  Render,
  Resize(u16, u16),
  Resume,
  Quit,
  Refresh,
  Error(String),
  Help,
  SubmitEditorQuery,
  SubmitEditorQueryBypassParser,
  TriggerCompletion,
  RequestExternalEditor,
  EditQueryExternally(Vec<String>),
  Query(Vec<String>, bool, bool), // (query_lines, execution_confirmed, bypass_parser)
  MenuPreview(MenuPreview, MenuTarget), // (preview, target)
  QueryToEditor(Vec<String>),
  ClearHistory,
  AbortQuery,
  FocusMenu,
  FocusEditor,
  FocusHistory,
  FocusData,
  FocusFavorites,
  CycleFocusForwards,
  CycleFocusBackwards,
  IncreaseSectionSize,
  DecreaseSectionSize,
  LoadMenu,
  CopyData(String),
  RequestExportData(i64),
  ExportData(ExportFormat),
  ExportDataFinished,
  RequestYankAll(i64),
  YankAll,
  RequestSaveFavorite(Vec<String>),
  SaveFavorite(String, Vec<String>),
  DeleteFavorite(String),
}
