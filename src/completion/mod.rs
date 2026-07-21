use std::{
  collections::{HashMap, HashSet},
  path::{Path, PathBuf},
  sync::{Arc, RwLock},
  time::Duration,
};

use sqlparser::{keywords::ALL_KEYWORDS, parser::Parser, tokenizer::Tokenizer};
use tokio::{sync::mpsc, task::JoinHandle};

use crate::database::{Database, Rows};
use crate::{cli::Driver, database::get_dialect};

pub mod render;

pub const MAX_CANDIDATES: usize = 50;

#[derive(Debug)]
pub enum CompletionCommand {
  Request(CompletionRequest),
  Cancel { generation: u64 },
}

#[derive(Debug)]
pub enum CompletionUiEvent {
  Response(CompletionResponse),
}

pub struct CompletionClient {
  pub command_tx: mpsc::UnboundedSender<CompletionCommand>,
  pub response_rx: mpsc::UnboundedReceiver<CompletionUiEvent>,
}

pub struct CompletionCoordinator {
  command_rx: mpsc::UnboundedReceiver<CompletionCommand>,
  response_tx: mpsc::UnboundedSender<CompletionUiEvent>,
  worker_tx: mpsc::UnboundedSender<CompletionResponse>,
  worker_rx: mpsc::UnboundedReceiver<CompletionResponse>,
  table_discovery_tx: mpsc::UnboundedSender<Vec<TableRef>>,
  table_discovery_rx: mpsc::UnboundedReceiver<Vec<TableRef>>,
  table_discovery_tasks: Vec<JoinHandle<()>>,
  pending_discovery_texts: HashSet<String>,
  catalog_loaded: bool,
  pending: Option<JoinHandle<()>>,
  latest_generation: u64,
  dismissed_generation: Option<u64>,
  catalog: Arc<RwLock<CompletionCatalog>>,
  latest_request: Option<CompletionRequest>,
  latest_trigger_length: usize,
  missing_columns: HashSet<TableRef>,
  in_flight_columns: HashSet<TableRef>,
  failed_columns: HashSet<TableRef>,
  menu_task: Option<JoinHandle<color_eyre::eyre::Result<Rows>>>,
  column_task: Option<JoinHandle<color_eyre::eyre::Result<Vec<TableColumns>>>>,
}

pub enum CompletionDatabaseEvent {
  MenuLoaded(Rows),
}

impl CompletionCoordinator {
  pub fn new() -> (Self, CompletionClient) {
    let (command_tx, command_rx) = mpsc::unbounded_channel();
    let (response_tx, response_rx) = mpsc::unbounded_channel();
    let (worker_tx, worker_rx) = mpsc::unbounded_channel();
    let (table_discovery_tx, table_discovery_rx) = mpsc::unbounded_channel();
    (
      Self {
        command_rx,
        response_tx,
        worker_tx,
        worker_rx,
        table_discovery_tx,
        table_discovery_rx,
        table_discovery_tasks: Vec::new(),
        pending_discovery_texts: HashSet::new(),
        catalog_loaded: false,
        pending: None,
        latest_generation: 0,
        dismissed_generation: None,
        catalog: Arc::new(RwLock::new(CompletionCatalog::default())),
        latest_request: None,
        latest_trigger_length: 2,
        missing_columns: HashSet::new(),
        in_flight_columns: HashSet::new(),
        failed_columns: HashSet::new(),
        menu_task: None,
        column_task: None,
      },
      CompletionClient { command_tx, response_rx },
    )
  }

  pub fn catalog(&self) -> Arc<RwLock<CompletionCatalog>> {
    self.catalog.clone()
  }

  pub fn poll(&mut self, debounce: Duration, trigger_length: usize) {
    while let Ok(command) = self.command_rx.try_recv() {
      match command {
        CompletionCommand::Request(request) => {
          self.queue_columns_for_text(request.text.clone());
          self.latest_generation = request.generation;
          if request.manual {
            self.dismissed_generation = None;
          }
          let delay = if request.manual { Duration::ZERO } else { debounce };
          self.latest_request = Some(request.clone());
          self.latest_trigger_length = trigger_length;
          self.spawn_completion(request, delay, trigger_length);
        },
        CompletionCommand::Cancel { generation } => {
          self.latest_generation = self.latest_generation.max(generation);
          self.dismissed_generation = Some(generation);
          if let Some(task) = self.pending.take() {
            task.abort();
          }
        },
      }
    }

    while let Ok(response) = self.worker_rx.try_recv() {
      if response.generation == self.latest_generation
        && self.dismissed_generation != Some(response.generation)
      {
        self.missing_columns.extend(
          response
            .missing_columns
            .iter()
            .filter(|table| {
              !self.in_flight_columns.contains(*table) && !self.failed_columns.contains(*table)
            })
            .cloned(),
        );
        let _ = self.response_tx.send(CompletionUiEvent::Response(response));
      }
    }

    while let Ok(tables) = self.table_discovery_rx.try_recv() {
      self.queue_missing_columns(tables);
    }
    self.table_discovery_tasks.retain(|task| !task.is_finished());
  }

  /// Finds every catalog table or view mentioned anywhere in raw query text and queues its
  /// columns. This intentionally ignores SQL lexical context, so comments and strings count too.
  pub fn queue_columns_for_text(&mut self, text: String) {
    if text.trim().is_empty() {
      return;
    }
    if !self.catalog_loaded {
      self.pending_discovery_texts.insert(text);
      return;
    }
    let catalog = self.catalog.clone();
    let result_tx = self.table_discovery_tx.clone();
    self.table_discovery_tasks.push(tokio::spawn(async move {
      let result = tokio::task::spawn_blocking(move || {
        let catalog = catalog.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        mentioned_table_refs(&text, &catalog)
      })
      .await;
      if let Ok(tables) = result {
        let _ = result_tx.send(tables);
      }
    }));
  }

  fn queue_missing_columns(&mut self, tables: Vec<TableRef>) {
    let catalog = self.catalog.read().unwrap_or_else(|poisoned| poisoned.into_inner());
    let missing: Vec<_> = tables
      .into_iter()
      .filter(|table| !catalog.columns.contains_key(table))
      .filter(|table| {
        !self.in_flight_columns.contains(table) && !self.failed_columns.contains(table)
      })
      .collect();
    drop(catalog);
    self.missing_columns.extend(missing);
  }

  fn spawn_completion(
    &mut self,
    request: CompletionRequest,
    delay: Duration,
    trigger_length: usize,
  ) {
    if let Some(task) = self.pending.take() {
      task.abort();
    }
    let catalog = self.catalog.clone();
    let worker_tx = self.worker_tx.clone();
    self.pending = Some(tokio::spawn(async move {
      tokio::time::sleep(delay).await;
      let analysis = analyze(&request);
      if !request.manual
        && !matches!(analysis.context, CompletionContext::StringLiteral { .. })
        && analysis.prefix.chars().count() < trigger_length
      {
        let _ = worker_tx.send(CompletionResponse {
          generation: request.generation,
          replacement_range: analysis.replacement_range,
          candidates: Vec::new(),
          missing_columns: Vec::new(),
        });
        return;
      }
      let path_candidates = path_candidates(&analysis).await;
      let response = tokio::task::spawn_blocking(move || {
        let catalog = catalog.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        complete(&request, &catalog, path_candidates)
      })
      .await;
      if let Ok(response) = response {
        let _ = worker_tx.send(response);
      }
    }));
  }

  pub fn start_menu_load(&mut self, database: &dyn Database) -> color_eyre::eyre::Result<()> {
    if let Some(task) = self.menu_task.take() {
      task.abort();
    }
    if let Some(task) = self.column_task.take() {
      task.abort();
    }
    for task in self.table_discovery_tasks.drain(..) {
      task.abort();
    }
    while self.table_discovery_rx.try_recv().is_ok() {}
    self.catalog_loaded = false;
    self.pending_discovery_texts.clear();
    if let Some(request) = &self.latest_request {
      self.pending_discovery_texts.insert(request.text.clone());
    }
    self.missing_columns.clear();
    self.in_flight_columns.clear();
    self.failed_columns.clear();
    *self.catalog.write().unwrap_or_else(|poisoned| poisoned.into_inner()) =
      CompletionCatalog::default();
    self.menu_task = Some(database.start_load_menu()?);
    Ok(())
  }

  pub fn start_missing_columns(&mut self, database: &dyn Database) -> color_eyre::eyre::Result<()> {
    if self.column_task.is_some() || self.missing_columns.is_empty() {
      return Ok(());
    }
    let tables: Vec<_> = self.missing_columns.drain().collect();
    self.in_flight_columns.extend(tables.iter().cloned());
    self.column_task = Some(database.start_load_columns(tables)?);
    Ok(())
  }

  pub async fn poll_database(&mut self) -> Vec<CompletionDatabaseEvent> {
    let mut events = Vec::new();
    if self.menu_task.as_ref().is_some_and(JoinHandle::is_finished) {
      let task = self.menu_task.take().unwrap();
      match task.await {
        Ok(Ok(rows)) => {
          let catalog = catalog_from_menu_rows(&rows);
          *self.catalog.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = catalog;
          self.catalog_loaded = true;
          events.push(CompletionDatabaseEvent::MenuLoaded(rows));
          for text in std::mem::take(&mut self.pending_discovery_texts) {
            self.queue_columns_for_text(text);
          }
          self.refresh_latest_completion();
        },
        Ok(Err(error)) => log::error!("failed to load completion catalog: {error}"),
        Err(error) if !error.is_cancelled() => {
          log::error!("completion catalog task failed: {error}")
        },
        Err(_) => {},
      }
    }
    if self.column_task.as_ref().is_some_and(JoinHandle::is_finished) {
      let task = self.column_task.take().unwrap();
      let requested: Vec<_> = self.in_flight_columns.drain().collect();
      match task.await {
        Ok(Ok(table_columns)) => {
          let mut catalog = self.catalog.write().unwrap_or_else(|poisoned| poisoned.into_inner());
          let loaded: HashSet<_> =
            table_columns.iter().map(|columns| columns.table.clone()).collect();
          for columns in table_columns {
            catalog.insert_columns(columns.table, columns.columns);
          }
          drop(catalog);
          self
            .failed_columns
            .extend(requested.iter().filter(|table| !loaded.contains(*table)).cloned());
          self.refresh_latest_completion();
        },
        Ok(Err(error)) => {
          log::warn!("failed to load completion columns: {error}");
          self.failed_columns.extend(requested);
        },
        Err(error) if !error.is_cancelled() => {
          log::warn!("completion column task failed: {error}");
          self.failed_columns.extend(requested);
        },
        Err(_) => {},
      }
    }
    events
  }

  fn refresh_latest_completion(&mut self) {
    if self.dismissed_generation != Some(self.latest_generation)
      && let Some(request) = self.latest_request.clone()
    {
      self.spawn_completion(request, Duration::ZERO, self.latest_trigger_length);
    }
  }

  pub fn cancel_all(&mut self) {
    if let Some(task) = self.pending.take() {
      task.abort();
    }
    if let Some(task) = self.menu_task.take() {
      task.abort();
    }
    if let Some(task) = self.column_task.take() {
      task.abort();
    }
    for task in self.table_discovery_tasks.drain(..) {
      task.abort();
    }
  }
}

impl Drop for CompletionCoordinator {
  fn drop(&mut self) {
    self.cancel_all();
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompletionKind {
  Keyword,
  Schema,
  Table,
  View,
  Function,
  Column,
  BufferWord,
  Path,
}

impl CompletionKind {
  pub const fn label(self) -> &'static str {
    match self {
      Self::Keyword => "keyword",
      Self::Schema => "schema",
      Self::Table => "table",
      Self::View => "view",
      Self::Function => "function",
      Self::Column => "column",
      Self::BufferWord => "buffer",
      Self::Path => "path",
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompletionSource {
  SqlSyntax,
  FileSystem,
  Buffer,
  Database,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionCandidate {
  pub label: String,
  pub insert_text: String,
  pub kind: CompletionKind,
  pub detail: Option<String>,
  pub source: CompletionSource,
}

impl CompletionCandidate {
  pub fn new(text: impl Into<String>, kind: CompletionKind, source: CompletionSource) -> Self {
    let text = text.into();
    Self { label: text.clone(), insert_text: text, kind, detail: None, source }
  }

  pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
    self.detail = Some(detail.into());
    self
  }

  pub fn with_insert_text(mut self, insert_text: impl Into<String>) -> Self {
    self.insert_text = insert_text.into();
    self
  }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CursorPosition {
  pub row: usize,
  pub col: usize,
}

impl From<(usize, usize)> for CursorPosition {
  fn from((row, col): (usize, usize)) -> Self {
    Self { row, col }
  }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextRange {
  pub start: CursorPosition,
  pub end: CursorPosition,
}

#[derive(Debug, Clone)]
pub struct CompletionRequest {
  pub generation: u64,
  pub text: String,
  pub cursor: CursorPosition,
  pub manual: bool,
  pub driver: Driver,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionResponse {
  pub generation: u64,
  pub replacement_range: TextRange,
  pub candidates: Vec<CompletionCandidate>,
  pub missing_columns: Vec<TableRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionContext {
  Comment,
  StringLiteral { fragment: String },
  Qualified { qualifier: String },
  Table,
  Expression,
  Generic,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TableRef {
  pub schema: String,
  pub table: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
  pub name: String,
  pub type_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableColumns {
  pub table: TableRef,
  pub columns: Vec<Column>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogObject {
  pub schema: String,
  pub name: String,
  pub kind: CompletionKind,
}

#[derive(Debug, Clone, Default)]
pub struct CompletionCatalog {
  pub objects: Vec<CatalogObject>,
  pub columns: HashMap<TableRef, Vec<Column>>,
}

impl CompletionCatalog {
  pub fn schemas(&self) -> impl Iterator<Item = &str> {
    let mut seen = HashSet::new();
    self.objects.iter().filter_map(move |object| {
      seen.insert(object.schema.as_str()).then_some(object.schema.as_str())
    })
  }

  pub fn insert_columns(&mut self, table: TableRef, columns: Vec<Column>) {
    self.columns.insert(table, columns);
  }
}

pub fn catalog_from_menu_rows(rows: &Rows) -> CompletionCatalog {
  let objects = rows
    .rows
    .iter()
    .filter_map(|row| {
      let schema = row.first()?.clone();
      let name = row.get(1)?.clone();
      let kind = match row.get(2).map(|kind| kind.to_ascii_lowercase()).as_deref() {
        Some("view" | "materialized_view" | "materialized view" | "mview") => CompletionKind::View,
        Some("function") => CompletionKind::Function,
        _ => CompletionKind::Table,
      };
      Some(CatalogObject { schema, name, kind })
    })
    .collect();
  CompletionCatalog { objects, columns: HashMap::new() }
}

pub fn mentioned_table_refs(text: &str, catalog: &CompletionCatalog) -> Vec<TableRef> {
  let text = text.to_lowercase();
  let mut tables = HashSet::new();
  for object in &catalog.objects {
    if !matches!(object.kind, CompletionKind::Table | CompletionKind::View) {
      continue;
    }
    let name = object.name.to_lowercase();
    if name.is_empty() {
      continue;
    }
    let mentioned = text.match_indices(&name).any(|(start, _)| {
      let end = start + name.len();
      let before = text[..start].chars().next_back();
      let after = text[end..].chars().next();
      before.is_none_or(|ch| !is_identifier_char(ch))
        && after.is_none_or(|ch| !is_identifier_char(ch))
    });
    if mentioned {
      tables.insert(TableRef { schema: object.schema.clone(), table: object.name.clone() });
    }
  }
  let mut tables: Vec<_> = tables.into_iter().collect();
  tables.sort();
  tables
}

pub fn table_columns_from_rows(table: TableRef, rows: Rows) -> TableColumns {
  let header_index = |names: &[&str]| {
    rows
      .headers
      .iter()
      .position(|header| names.iter().any(|name| header.name.eq_ignore_ascii_case(name)))
  };
  let name_index = header_index(&["column_name", "name"]).unwrap_or(0);
  let type_index = header_index(&["data_type", "type", "type_name"]);
  let columns = rows
    .rows
    .into_iter()
    .filter_map(|row| {
      let name = row.get(name_index)?.clone();
      let type_name = type_index.and_then(|index| row.get(index).cloned()).unwrap_or_default();
      Some(Column { name, type_name })
    })
    .collect();
  TableColumns { table, columns }
}

#[derive(Debug, Clone)]
pub struct Analysis {
  pub context: CompletionContext,
  pub prefix: String,
  pub replacement_range: TextRange,
  pub aliases: HashMap<String, TableRef>,
  pub referenced_tables: Vec<TableRef>,
  pub ctes: Vec<String>,
}

pub fn analyze(request: &CompletionRequest) -> Analysis {
  let lines: Vec<&str> = request.text.split('\n').collect();
  let row = request.cursor.row.min(lines.len().saturating_sub(1));
  let line = lines.get(row).copied().unwrap_or("");
  let col = request.cursor.col.min(line.chars().count());
  let byte_col = char_to_byte(line, col);
  let before_line = &line[..byte_col];
  let before_cursor = text_before_cursor(&request.text, row, col);
  let lexical = lexical_state(&before_cursor);

  if lexical.in_line_comment || lexical.in_block_comment {
    return Analysis {
      context: CompletionContext::Comment,
      prefix: String::new(),
      replacement_range: TextRange {
        start: CursorPosition { row, col },
        end: CursorPosition { row, col },
      },
      aliases: HashMap::new(),
      referenced_tables: Vec::new(),
      ctes: Vec::new(),
    };
  }

  if lexical.in_single_quote {
    let fragment_start = before_line.rfind('\'').map_or(0, |idx| idx + 1);
    let fragment_text = &before_line[fragment_start..];
    let path_start =
      fragment_text.rfind(['/', '\\']).map_or(fragment_start, |idx| fragment_start + idx + 1);
    let start_col = line[..path_start].chars().count();
    return Analysis {
      context: CompletionContext::StringLiteral { fragment: fragment_text.to_owned() },
      prefix: before_line[path_start..].to_owned(),
      replacement_range: TextRange {
        start: CursorPosition { row, col: start_col },
        end: CursorPosition { row, col },
      },
      aliases: HashMap::new(),
      referenced_tables: Vec::new(),
      ctes: Vec::new(),
    };
  }

  let prefix_start = before_line
    .char_indices()
    .rev()
    .find(|(_, ch)| !is_identifier_char(*ch))
    .map_or(0, |(idx, ch)| idx + ch.len_utf8());
  let prefix = before_line[prefix_start..].to_owned();
  let start_col = line[..prefix_start].chars().count();
  let replacement_range =
    TextRange { start: CursorPosition { row, col: start_col }, end: CursorPosition { row, col } };

  let statement = current_statement(&before_cursor);
  let tokens = syntax_tokens(statement, request.driver);
  let (aliases, referenced_tables) = extract_tables(&tokens);
  let ctes = extract_ctes(&tokens);

  let qualifier = before_line[..prefix_start].strip_suffix('.').and_then(|before_dot| {
    let start = before_dot
      .char_indices()
      .rev()
      .find(|(_, ch)| !is_identifier_char(*ch))
      .map_or(0, |(idx, ch)| idx + ch.len_utf8());
    let qualifier = &before_dot[start..];
    (!qualifier.is_empty()).then(|| qualifier.to_owned())
  });

  let context = if let Some(qualifier) = qualifier {
    CompletionContext::Qualified { qualifier }
  } else {
    classify_context(&tokens, &prefix)
  };

  Analysis { context, prefix, replacement_range, aliases, referenced_tables, ctes }
}

pub fn complete(
  request: &CompletionRequest,
  catalog: &CompletionCatalog,
  path_candidates: Vec<CompletionCandidate>,
) -> CompletionResponse {
  let analysis = analyze(request);
  let mut candidates = Vec::new();
  let mut missing_columns = Vec::new();
  let mut preferred_column_details = HashSet::new();
  log::debug!("completion analysis: {analysis:?}");

  match &analysis.context {
    CompletionContext::Comment => {},
    CompletionContext::StringLiteral { .. } => candidates.extend(path_candidates),
    CompletionContext::Qualified { qualifier } => {
      if catalog.schemas().any(|schema| schema.eq_ignore_ascii_case(qualifier)) {
        candidates.extend(
          catalog
            .objects
            .iter()
            .filter(|object| object.schema.eq_ignore_ascii_case(qualifier))
            .map(|object| database_object_candidate(object, request.driver)),
        );
      } else if let Some(table) = resolve_table(qualifier, &analysis, catalog) {
        if let Some(columns) = catalog.columns.get(&table) {
          candidates
            .extend(columns.iter().map(|column| column_candidate(column, &table, request.driver)));
        } else {
          missing_columns.push(table);
        }
      }
    },
    CompletionContext::Table => {
      candidates.extend(catalog.schemas().map(|schema| {
        identifier_candidate(
          schema,
          CompletionKind::Schema,
          CompletionSource::Database,
          request.driver,
        )
      }));
      candidates.extend(
        catalog
          .objects
          .iter()
          .filter(|object| matches!(object.kind, CompletionKind::Table | CompletionKind::View))
          .map(|object| database_object_candidate(object, request.driver)),
      );
      candidates.extend(analysis.ctes.iter().map(|cte| {
        identifier_candidate(cte, CompletionKind::Table, CompletionSource::Buffer, request.driver)
          .with_detail("CTE")
      }));
      add_keywords(&mut candidates);
    },
    CompletionContext::Expression => {
      let mut tables: HashSet<_> =
        analysis.referenced_tables.iter().map(|table| resolved_table_ref(table, catalog)).collect();
      tables.extend(mentioned_table_refs(&request.text, catalog));
      add_unqualified_columns(
        &mut candidates,
        &mut missing_columns,
        &mut preferred_column_details,
        catalog,
        tables,
        request.driver,
      );
      add_buffer_words(&mut candidates, &request.text);
      add_keywords(&mut candidates);
      candidates.extend(
        catalog.objects.iter().map(|object| database_object_candidate(object, request.driver)),
      );
    },
    CompletionContext::Generic => {
      add_unqualified_columns(
        &mut candidates,
        &mut missing_columns,
        &mut preferred_column_details,
        catalog,
        mentioned_table_refs(&request.text, catalog).into_iter().collect(),
        request.driver,
      );
      add_keywords(&mut candidates);
      add_buffer_words(&mut candidates, &request.text);
      candidates.extend(
        catalog.objects.iter().map(|object| database_object_candidate(object, request.driver)),
      );
    },
  }

  let candidates =
    rank_candidates(candidates, &analysis.prefix, &analysis.context, &preferred_column_details);
  CompletionResponse {
    generation: request.generation,
    replacement_range: analysis.replacement_range,
    candidates,
    missing_columns,
  }
}

pub fn extract_words(text: &str) -> Vec<String> {
  let keywords: HashSet<&str> = ALL_KEYWORDS.iter().copied().collect();
  let mut words = HashSet::new();
  let mut current = String::new();
  for ch in text.chars().chain(std::iter::once(' ')) {
    if is_identifier_char(ch) {
      current.push(ch);
    } else {
      if current.chars().count() >= 2 && !keywords.contains(current.to_ascii_uppercase().as_str()) {
        words.insert(current.clone());
      }
      current.clear();
    }
  }
  let mut words: Vec<_> = words.into_iter().collect();
  words.sort_by_key(|word| word.to_lowercase());
  words
}

pub fn current_replacement_range(text: &str, cursor: CursorPosition) -> TextRange {
  let request = CompletionRequest {
    generation: 0,
    text: text.to_owned(),
    cursor,
    manual: true,
    driver: Driver::Postgres,
  };
  analyze(&request).replacement_range
}

async fn path_candidates(analysis: &Analysis) -> Vec<CompletionCandidate> {
  let CompletionContext::StringLiteral { fragment } = &analysis.context else {
    return Vec::new();
  };
  let separator = fragment.rfind(['/', '\\']);
  let (directory_fragment, basename) = separator
    .map_or(("", fragment.as_str()), |index| (&fragment[..=index], &fragment[index + 1..]));
  let directory = expand_directory(directory_fragment);
  let Ok(mut entries) = tokio::fs::read_dir(&directory).await else {
    return Vec::new();
  };
  let show_hidden = basename.starts_with('.');
  let basename_lower = basename.to_lowercase();
  let mut candidates = Vec::new();
  while let Ok(Some(entry)) = entries.next_entry().await {
    let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
    if name.starts_with('.') && !show_hidden {
      continue;
    }
    let lower = name.to_lowercase();
    if !basename_lower.is_empty() && !lower.contains(&basename_lower) {
      continue;
    }
    let is_dir = entry.file_type().await.is_ok_and(|kind| kind.is_dir());
    let insert_text = if is_dir { format!("{name}/") } else { name.clone() };
    candidates.push(
      CompletionCandidate::new(name, CompletionKind::Path, CompletionSource::FileSystem)
        .with_insert_text(insert_text)
        .with_detail(if is_dir { "directory" } else { "file" }),
    );
  }
  candidates
}

fn expand_directory(fragment: &str) -> PathBuf {
  if fragment.is_empty() {
    return std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
  }
  if (fragment == "~/" || fragment.starts_with("~/"))
    && let Some(home) = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
  {
    return home.join(fragment.trim_start_matches("~/"));
  }
  let path = Path::new(fragment);
  if path.is_absolute() {
    path.to_path_buf()
  } else {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(path)
  }
}

fn add_keywords(candidates: &mut Vec<CompletionCandidate>) {
  candidates.extend(ALL_KEYWORDS.iter().map(|keyword| {
    CompletionCandidate::new(*keyword, CompletionKind::Keyword, CompletionSource::SqlSyntax)
  }));
}

fn add_buffer_words(candidates: &mut Vec<CompletionCandidate>, text: &str) {
  candidates.extend(extract_words(text).into_iter().map(|word| {
    CompletionCandidate::new(word, CompletionKind::BufferWord, CompletionSource::Buffer)
  }));
}

fn identifier_candidate(
  identifier: &str,
  kind: CompletionKind,
  source: CompletionSource,
  driver: Driver,
) -> CompletionCandidate {
  CompletionCandidate::new(identifier, kind, source)
    .with_insert_text(identifier_insert_text(identifier, driver))
}

fn identifier_insert_text(identifier: &str, driver: Driver) -> String {
  if !identifier.chars().any(char::is_whitespace) {
    return identifier.to_owned();
  }
  match driver {
    Driver::MySql => format!("`{}`", identifier.replace('`', "``")),
    Driver::Postgres | Driver::Sqlite | Driver::Oracle => {
      format!("\"{}\"", identifier.replace('"', "\"\""))
    },
    #[cfg(feature = "duckdb")]
    Driver::DuckDb => format!("\"{}\"", identifier.replace('"', "\"\"")),
  }
}

fn database_object_candidate(object: &CatalogObject, driver: Driver) -> CompletionCandidate {
  let candidate = if matches!(object.kind, CompletionKind::Table | CompletionKind::View) {
    identifier_candidate(&object.name, object.kind, CompletionSource::Database, driver)
  } else {
    CompletionCandidate::new(&object.name, object.kind, CompletionSource::Database)
  };
  candidate.with_detail(&object.schema)
}

fn column_candidate(column: &Column, table: &TableRef, driver: Driver) -> CompletionCandidate {
  let detail = if column.type_name.is_empty() {
    format!("{}.{}", table.schema, table.table)
  } else {
    format!("{} · {}.{}", column.type_name, table.schema, table.table)
  };
  identifier_candidate(&column.name, CompletionKind::Column, CompletionSource::Database, driver)
    .with_detail(detail)
}

fn add_unqualified_columns(
  candidates: &mut Vec<CompletionCandidate>,
  missing_columns: &mut Vec<TableRef>,
  preferred_column_details: &mut HashSet<String>,
  catalog: &CompletionCatalog,
  preferred_tables: HashSet<TableRef>,
  driver: Driver,
) {
  let mut preferred_tables: Vec<_> = preferred_tables.into_iter().collect();
  preferred_tables.sort();
  for table in &preferred_tables {
    if let Some(columns) = catalog.columns.get(table) {
      for column in columns {
        let candidate = column_candidate(column, table, driver);
        if let Some(detail) = &candidate.detail {
          preferred_column_details.insert(detail.clone());
        }
        candidates.push(candidate);
      }
    } else if !missing_columns.contains(table) {
      missing_columns.push(table.clone());
    }
  }

  let preferred_tables: HashSet<_> = preferred_tables.into_iter().collect();
  let mut other_cached_tables: Vec<_> =
    catalog.columns.keys().filter(|table| !preferred_tables.contains(*table)).collect();
  other_cached_tables.sort();
  for table in other_cached_tables {
    if let Some(columns) = catalog.columns.get(table) {
      candidates.extend(columns.iter().map(|column| column_candidate(column, table, driver)));
    }
  }
}

fn resolve_table(
  qualifier: &str,
  analysis: &Analysis,
  catalog: &CompletionCatalog,
) -> Option<TableRef> {
  if let Some(table) = analysis.aliases.get(&qualifier.to_ascii_lowercase()) {
    return Some(resolved_table_ref(table, catalog));
  }
  analysis
    .referenced_tables
    .iter()
    .find(|table| table.table.eq_ignore_ascii_case(qualifier))
    .map(|table| resolved_table_ref(table, catalog))
    .or_else(|| {
      catalog
        .objects
        .iter()
        .find(|object| {
          matches!(object.kind, CompletionKind::Table | CompletionKind::View)
            && object.name.eq_ignore_ascii_case(qualifier)
        })
        .map(|object| TableRef { schema: object.schema.clone(), table: object.name.clone() })
    })
}

fn resolved_table_ref(table: &TableRef, catalog: &CompletionCatalog) -> TableRef {
  if !table.schema.is_empty() {
    return table.clone();
  }
  catalog
    .objects
    .iter()
    .find(|object| {
      matches!(object.kind, CompletionKind::Table | CompletionKind::View)
        && object.name.eq_ignore_ascii_case(&table.table)
    })
    .map(|object| TableRef { schema: object.schema.clone(), table: object.name.clone() })
    .or_else(|| {
      catalog
        .columns
        .keys()
        .find(|candidate| candidate.table.eq_ignore_ascii_case(&table.table))
        .cloned()
    })
    .unwrap_or_else(|| table.clone())
}

fn rank_candidates(
  candidates: Vec<CompletionCandidate>,
  prefix: &str,
  context: &CompletionContext,
  preferred_column_details: &HashSet<String>,
) -> Vec<CompletionCandidate> {
  let prefix = prefix.to_lowercase();
  let mut best: HashMap<String, (u8, u8, u8, CompletionCandidate)> = HashMap::new();
  for candidate in candidates {
    let text = candidate.label.to_lowercase();
    let match_tier = if prefix.is_empty() || text.starts_with(&prefix) {
      0
    } else if text.contains(&prefix) {
      1
    } else {
      continue;
    };
    let source_tier = source_priority(candidate.kind, context);
    let preferred_tier = u8::from(
      candidate.kind != CompletionKind::Column
        || candidate
          .detail
          .as_ref()
          .is_none_or(|detail| !preferred_column_details.contains(detail)),
    );
    let key = candidate.insert_text.to_lowercase();
    let score = (source_tier, preferred_tier, match_tier);
    if best
      .get(&key)
      .is_none_or(|(source, preferred, matched, _)| score < (*source, *preferred, *matched))
    {
      best.insert(key, (source_tier, preferred_tier, match_tier, candidate));
    }
  }
  let mut ranked: Vec<_> = best.into_values().collect();
  ranked.sort_by(|a, b| {
    (a.0, a.1, a.2, a.3.label.to_lowercase()).cmp(&(b.0, b.1, b.2, b.3.label.to_lowercase()))
  });
  ranked.into_iter().map(|(_, _, _, candidate)| candidate).take(MAX_CANDIDATES).collect()
}

fn source_priority(kind: CompletionKind, context: &CompletionContext) -> u8 {
  match context {
    CompletionContext::StringLiteral { .. } => 0,
    CompletionContext::Qualified { .. } => match kind {
      CompletionKind::Column | CompletionKind::Table | CompletionKind::View => 0,
      _ => 3,
    },
    CompletionContext::Table => match kind {
      CompletionKind::Schema | CompletionKind::Table | CompletionKind::View => 0,
      CompletionKind::BufferWord => 1,
      _ => 2,
    },
    CompletionContext::Expression => match kind {
      CompletionKind::Column => 0,
      CompletionKind::BufferWord => 1,
      CompletionKind::Keyword => 2,
      _ => 3,
    },
    CompletionContext::Generic => match kind {
      CompletionKind::Column => 0,
      CompletionKind::Keyword => 1,
      CompletionKind::BufferWord => 2,
      _ => 3,
    },
    CompletionContext::Comment => 9,
  }
}

fn classify_context(tokens: &[String], prefix: &str) -> CompletionContext {
  let mut significant = tokens.to_vec();
  if !prefix.is_empty() && significant.last().is_some_and(|last| last.eq_ignore_ascii_case(prefix))
  {
    significant.pop();
  }
  let words: Vec<String> = significant
    .iter()
    .filter(|token| token.chars().any(is_identifier_char))
    .map(|token| token.to_ascii_uppercase())
    .collect();
  let last = words.last().map(String::as_str).unwrap_or("");
  if matches!(
    last,
    "FROM" | "JOIN" | "INTO" | "UPDATE" | "TABLE" | "VIEW" | "TRUNCATE" | "DESC" | "DESCRIBE"
  ) {
    return CompletionContext::Table;
  }
  if words
    .iter()
    .rev()
    .any(|word| matches!(word.as_str(), "SELECT" | "WHERE" | "ON" | "HAVING" | "SET" | "RETURNING"))
  {
    return CompletionContext::Expression;
  }
  CompletionContext::Generic
}

fn syntax_tokens(statement: &str, driver: Driver) -> Vec<String> {
  let dialect = get_dialect(driver);
  if Parser::parse_sql(&*dialect, statement).is_ok() {
    // Parsing is intentionally best-effort. Token positions remain useful for both valid and
    // incomplete statements, while a successful parse confirms dialect-specific syntax.
  }
  Tokenizer::new(&*dialect, statement)
    .tokenize()
    .map(|tokens| {
      tokens
        .into_iter()
        .map(|token| token.to_string())
        .filter(|token| !token.trim().is_empty())
        .collect()
    })
    .unwrap_or_else(|_| permissive_tokens(statement))
}

fn permissive_tokens(statement: &str) -> Vec<String> {
  let mut tokens = Vec::new();
  let mut word = String::new();
  for ch in statement.chars().chain(std::iter::once(' ')) {
    if is_identifier_char(ch) {
      word.push(ch);
    } else {
      if !word.is_empty() {
        tokens.push(std::mem::take(&mut word));
      }
      if matches!(ch, '.' | ',' | '(' | ')') {
        tokens.push(ch.to_string());
      }
    }
  }
  tokens
}

fn extract_tables(tokens: &[String]) -> (HashMap<String, TableRef>, Vec<TableRef>) {
  let mut aliases = HashMap::new();
  let mut tables = Vec::new();
  let mut i = 0;
  while i < tokens.len() {
    if !matches!(tokens[i].to_ascii_uppercase().as_str(), "FROM" | "JOIN" | "UPDATE" | "INTO") {
      i += 1;
      continue;
    }
    i += 1;
    let Some(first) = tokens.get(i).filter(|token| is_identifier(token)) else { continue };
    let mut schema = String::new();
    let mut table = first.clone();
    if tokens.get(i + 1).is_some_and(|token| token == ".")
      && let Some(name) = tokens.get(i + 2).filter(|token| is_identifier(token))
    {
      schema = table;
      table = name.clone();
      i += 2;
    }
    let table_ref = TableRef { schema, table };
    if !tables.contains(&table_ref) {
      tables.push(table_ref.clone());
    }
    i += 1;
    if tokens.get(i).is_some_and(|token| token.eq_ignore_ascii_case("AS")) {
      i += 1;
    }
    if let Some(alias) = tokens.get(i).filter(|token| is_identifier(token))
      && !is_clause_keyword(alias)
    {
      aliases.insert(alias.to_ascii_lowercase(), table_ref);
    }
  }
  (aliases, tables)
}

fn extract_ctes(tokens: &[String]) -> Vec<String> {
  let mut ctes = Vec::new();
  let Some(with_index) = tokens.iter().position(|token| token.eq_ignore_ascii_case("WITH")) else {
    return ctes;
  };
  for window in tokens[with_index + 1..].windows(2) {
    if is_identifier(&window[0]) && window[1].eq_ignore_ascii_case("AS") {
      ctes.push(window[0].clone());
    }
  }
  ctes
}

fn current_statement(before_cursor: &str) -> &str {
  before_cursor.rsplit_once(';').map_or(before_cursor, |(_, statement)| statement)
}

fn text_before_cursor(text: &str, row: usize, col: usize) -> String {
  let mut out = String::new();
  for (index, line) in text.split('\n').enumerate() {
    if index > row {
      break;
    }
    if index > 0 {
      out.push('\n');
    }
    if index == row {
      out.extend(line.chars().take(col));
      break;
    }
    out.push_str(line);
  }
  out
}

#[derive(Default)]
struct LexicalState {
  in_single_quote: bool,
  in_line_comment: bool,
  in_block_comment: bool,
}

fn lexical_state(text: &str) -> LexicalState {
  let chars: Vec<char> = text.chars().collect();
  let mut state = LexicalState::default();
  let mut i = 0;
  while i < chars.len() {
    if state.in_line_comment {
      if chars[i] == '\n' {
        state.in_line_comment = false;
      }
      i += 1;
      continue;
    }
    if state.in_block_comment {
      if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
        state.in_block_comment = false;
        i += 2;
      } else {
        i += 1;
      }
      continue;
    }
    if state.in_single_quote {
      if chars[i] == '\'' {
        if chars.get(i + 1) == Some(&'\'') {
          i += 2;
        } else {
          state.in_single_quote = false;
          i += 1;
        }
      } else {
        i += 1;
      }
      continue;
    }
    match (chars[i], chars.get(i + 1)) {
      ('-', Some('-')) => {
        state.in_line_comment = true;
        i += 2;
      },
      ('/', Some('*')) => {
        state.in_block_comment = true;
        i += 2;
      },
      ('\'', _) => {
        state.in_single_quote = true;
        i += 1;
      },
      _ => i += 1,
    }
  }
  state
}

fn char_to_byte(text: &str, char_index: usize) -> usize {
  text.char_indices().nth(char_index).map_or(text.len(), |(index, _)| index)
}

fn is_identifier_char(ch: char) -> bool {
  ch.is_alphanumeric() || matches!(ch, '_' | '$')
}

fn is_identifier(token: &str) -> bool {
  !token.is_empty() && token.chars().all(is_identifier_char)
}

fn is_clause_keyword(token: &str) -> bool {
  matches!(
    token.to_ascii_uppercase().as_str(),
    "WHERE"
      | "JOIN"
      | "LEFT"
      | "RIGHT"
      | "INNER"
      | "OUTER"
      | "FULL"
      | "CROSS"
      | "ON"
      | "GROUP"
      | "ORDER"
      | "LIMIT"
      | "HAVING"
      | "RETURNING"
      | "SET"
      | "VALUES"
  )
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::database::Header;

  fn request(text: &str) -> CompletionRequest {
    CompletionRequest {
      generation: 1,
      text: text.to_owned(),
      cursor: CursorPosition {
        row: text.matches('\n').count(),
        col: text.lines().last().unwrap_or("").chars().count(),
      },
      manual: false,
      driver: Driver::Postgres,
    }
  }

  #[test]
  fn detects_incomplete_table_context() {
    let analysis = analyze(&request("select * from us"));
    assert_eq!(analysis.context, CompletionContext::Table);
    assert_eq!(analysis.prefix, "us");
  }

  #[test]
  fn detects_alias_qualified_columns() {
    let analysis = analyze(&request("select u.na from public.users as u"));
    assert_eq!(analysis.context, CompletionContext::Expression);

    let analysis = analyze(&request("select * from public.users as u where u.na"));
    assert_eq!(analysis.context, CompletionContext::Qualified { qualifier: "u".into() });
    assert_eq!(analysis.aliases["u"].table, "users");
    assert_eq!(analysis.aliases["u"].schema, "public");
  }

  #[test]
  fn extracts_ctes() {
    let analysis = analyze(&request("with recent as (select 1) select * from re"));
    assert!(analysis.ctes.contains(&"recent".to_owned()));
  }

  #[test]
  fn suppresses_comments() {
    assert_eq!(analyze(&request("select 1 -- sel")).context, CompletionContext::Comment);
    assert_eq!(analyze(&request("select /* sel")).context, CompletionContext::Comment);
  }

  #[test]
  fn detects_string_path_fragment() {
    let analysis = analyze(&request("select './fixtures/us"));
    assert_eq!(
      analysis.context,
      CompletionContext::StringLiteral { fragment: "./fixtures/us".into() }
    );
    assert_eq!(analysis.prefix, "us");
  }

  #[test]
  fn replacement_range_is_character_based() {
    let analysis = analyze(&request("select café"));
    assert_eq!(analysis.replacement_range.start.col, 7);
    assert_eq!(analysis.replacement_range.end.col, 11);
  }

  #[test]
  fn extracts_unicode_buffer_words_and_excludes_keywords() {
    assert_eq!(extract_words("SELECT café café 東京 x"), vec!["café", "東京"]);
  }

  #[test]
  fn ranks_context_source_then_prefix_and_deduplicates() {
    let candidates = vec![
      CompletionCandidate::new("users", CompletionKind::BufferWord, CompletionSource::Buffer),
      CompletionCandidate::new("other_users", CompletionKind::Table, CompletionSource::Database),
      CompletionCandidate::new("users", CompletionKind::Table, CompletionSource::Database),
    ];
    let ranked = rank_candidates(candidates, "us", &CompletionContext::Table, &HashSet::new());
    assert_eq!(
      ranked.iter().map(|item| item.label.as_str()).collect::<Vec<_>>(),
      vec!["users", "other_users"]
    );
    assert_eq!(ranked[0].kind, CompletionKind::Table);
  }

  #[test]
  fn quotes_whitespace_identifiers_only_in_insertion_text() {
    let table = TableRef { schema: "sales data".into(), table: "order details".into() };
    let mut catalog = CompletionCatalog {
      objects: vec![CatalogObject {
        schema: table.schema.clone(),
        name: table.table.clone(),
        kind: CompletionKind::Table,
      }],
      ..CompletionCatalog::default()
    };
    catalog
      .insert_columns(table, vec![Column { name: "full name".into(), type_name: "text".into() }]);

    let table_response = complete(&request("select * from "), &catalog, Vec::new());
    let schema = table_response
      .candidates
      .iter()
      .find(|candidate| candidate.kind == CompletionKind::Schema)
      .unwrap();
    assert_eq!(schema.label, "sales data");
    assert_eq!(schema.insert_text, "\"sales data\"");
    let table = table_response
      .candidates
      .iter()
      .find(|candidate| candidate.kind == CompletionKind::Table)
      .unwrap();
    assert_eq!(table.label, "order details");
    assert_eq!(table.insert_text, "\"order details\"");

    let column_response = complete(&request("select fu"), &catalog, Vec::new());
    let column = column_response
      .candidates
      .iter()
      .find(|candidate| candidate.kind == CompletionKind::Column)
      .unwrap();
    assert_eq!(column.label, "full name");
    assert_eq!(column.insert_text, "\"full name\"");
  }

  #[test]
  fn uses_mysql_identifier_quotes_and_escapes_quote_delimiters() {
    assert_eq!(identifier_insert_text("order details", Driver::MySql), "`order details`");
    assert_eq!(identifier_insert_text("odd ` name", Driver::MySql), "`odd `` name`");
    assert_eq!(identifier_insert_text("odd \" name", Driver::Postgres), "\"odd \"\" name\"");
    assert_eq!(identifier_insert_text("ordinary_name", Driver::Postgres), "ordinary_name");
  }

  #[test]
  fn requests_missing_columns_for_qualified_table() {
    let catalog = CompletionCatalog {
      objects: vec![CatalogObject {
        schema: "public".into(),
        name: "users".into(),
        kind: CompletionKind::Table,
      }],
      ..CompletionCatalog::default()
    };
    let response = complete(&request("select users.na"), &catalog, Vec::new());
    assert_eq!(
      response.missing_columns,
      vec![TableRef { schema: "public".into(), table: "users".into() }]
    );
  }

  #[test]
  fn returns_cached_columns_for_qualified_table() {
    let table = TableRef { schema: "public".into(), table: "users".into() };
    let mut catalog = CompletionCatalog {
      objects: vec![CatalogObject {
        schema: table.schema.clone(),
        name: table.table.clone(),
        kind: CompletionKind::Table,
      }],
      ..CompletionCatalog::default()
    };
    catalog.insert_columns(
      table,
      vec![
        Column { name: "id".into(), type_name: "integer".into() },
        Column { name: "name".into(), type_name: "text".into() },
      ],
    );

    let response = complete(&request("select users.na"), &catalog, Vec::new());
    assert_eq!(response.missing_columns, Vec::<TableRef>::new());
    assert_eq!(response.candidates.len(), 1);
    assert_eq!(response.candidates[0].label, "name");
    assert_eq!(response.candidates[0].kind, CompletionKind::Column);
    assert_eq!(response.candidates[0].detail.as_deref(), Some("text · public.users"));
  }

  #[test]
  fn returns_all_cached_columns_in_generic_context() {
    let users = TableRef { schema: "public".into(), table: "users".into() };
    let orders = TableRef { schema: "public".into(), table: "orders".into() };
    let mut catalog = CompletionCatalog {
      objects: vec![
        CatalogObject {
          schema: users.schema.clone(),
          name: users.table.clone(),
          kind: CompletionKind::Table,
        },
        CatalogObject {
          schema: orders.schema.clone(),
          name: orders.table.clone(),
          kind: CompletionKind::Table,
        },
      ],
      ..CompletionCatalog::default()
    };
    catalog.insert_columns(users, vec![Column { name: "name".into(), type_name: "text".into() }]);
    catalog
      .insert_columns(orders, vec![Column { name: "number".into(), type_name: "integer".into() }]);

    let response = complete(&request("users; "), &catalog, Vec::new());
    assert!(
      response
        .candidates
        .iter()
        .any(|candidate| { candidate.label == "name" && candidate.kind == CompletionKind::Column })
    );
    assert!(response.candidates.iter().any(|candidate| {
      candidate.label == "number" && candidate.kind == CompletionKind::Column
    }));
  }

  #[test]
  fn returns_columns_from_table_mentioned_after_cursor_in_expression_context() {
    let users = TableRef { schema: "public".into(), table: "users".into() };
    let mut catalog = CompletionCatalog {
      objects: vec![CatalogObject {
        schema: users.schema.clone(),
        name: users.table.clone(),
        kind: CompletionKind::Table,
      }],
      ..CompletionCatalog::default()
    };
    catalog.insert_columns(users, vec![Column { name: "name".into(), type_name: "text".into() }]);
    let request = CompletionRequest {
      generation: 1,
      text: "select na from users".into(),
      cursor: CursorPosition { row: 0, col: 9 },
      manual: false,
      driver: Driver::Postgres,
    };

    assert_eq!(analyze(&request).context, CompletionContext::Expression);
    let response = complete(&request, &catalog, Vec::new());
    assert!(
      response
        .candidates
        .iter()
        .any(|candidate| { candidate.label == "name" && candidate.kind == CompletionKind::Column })
    );
  }

  #[test]
  fn returns_cached_columns_without_a_table_mention_in_expression_context() {
    let users = TableRef { schema: "public".into(), table: "users".into() };
    let mut catalog = CompletionCatalog::default();
    catalog.insert_columns(users, vec![Column { name: "name".into(), type_name: "text".into() }]);

    let response = complete(&request("select na"), &catalog, Vec::new());

    assert_eq!(analyze(&request("select na")).context, CompletionContext::Expression);
    assert!(
      response
        .candidates
        .iter()
        .any(|candidate| candidate.label == "name" && candidate.kind == CompletionKind::Column)
    );
  }

  #[test]
  fn prioritizes_columns_from_mentioned_tables_over_other_cached_columns() {
    let users = TableRef { schema: "public".into(), table: "users".into() };
    let orders = TableRef { schema: "public".into(), table: "orders".into() };
    let mut catalog = CompletionCatalog {
      objects: vec![CatalogObject {
        schema: users.schema.clone(),
        name: users.table.clone(),
        kind: CompletionKind::Table,
      }],
      ..CompletionCatalog::default()
    };
    catalog
      .insert_columns(users, vec![Column { name: "z_mentioned".into(), type_name: "text".into() }]);
    catalog.insert_columns(
      orders,
      vec![Column { name: "a_unmentioned".into(), type_name: "text".into() }],
    );
    let request = CompletionRequest {
      generation: 1,
      text: "select  from users".into(),
      cursor: CursorPosition { row: 0, col: 7 },
      manual: true,
      driver: Driver::Postgres,
    };

    let response = complete(&request, &catalog, Vec::new());
    let columns: Vec<_> = response
      .candidates
      .iter()
      .filter(|candidate| candidate.kind == CompletionKind::Column)
      .map(|candidate| candidate.label.as_str())
      .collect();

    assert_eq!(columns, vec!["z_mentioned", "a_unmentioned"]);
  }

  #[test]
  fn buffer_candidates_do_not_include_external_text() {
    let response =
      complete(&request("select local_word where lo"), &CompletionCatalog::default(), Vec::new());
    assert!(response.candidates.iter().any(|candidate| candidate.label == "local_word"));
    assert!(!response.candidates.iter().any(|candidate| candidate.label == "history_word"));
  }

  #[test]
  fn normalizes_menu_and_column_rows() {
    let menu = Rows {
      headers: vec![],
      rows: vec![
        vec!["public".into(), "users".into(), "table".into()],
        vec!["public".into(), "active_users".into(), "view".into()],
      ],
      rows_affected: None,
    };
    let catalog = catalog_from_menu_rows(&menu);
    assert_eq!(catalog.objects[0].kind, CompletionKind::Table);
    assert_eq!(catalog.objects[1].kind, CompletionKind::View);

    let table = TableRef { schema: "public".into(), table: "users".into() };
    let columns = table_columns_from_rows(
      table.clone(),
      Rows {
        headers: vec![
          Header { name: "COLUMN_NAME".into(), type_name: "text".into() },
          Header { name: "DATA_TYPE".into(), type_name: "text".into() },
        ],
        rows: vec![vec!["id".into(), "integer".into()], vec!["name".into(), "text".into()]],
        rows_affected: None,
      },
    );
    assert_eq!(columns.table, table);
    assert_eq!(columns.columns[0], Column { name: "id".into(), type_name: "integer".into() });
    assert_eq!(columns.columns[1].name, "name");
  }

  #[test]
  fn finds_catalog_tables_anywhere_in_raw_text() {
    let catalog = CompletionCatalog {
      objects: vec![
        CatalogObject {
          schema: "public".into(),
          name: "users".into(),
          kind: CompletionKind::Table,
        },
        CatalogObject { schema: "audit".into(), name: "users".into(), kind: CompletionKind::View },
        CatalogObject { schema: "public".into(), name: "user".into(), kind: CompletionKind::Table },
        CatalogObject {
          schema: "public".into(),
          name: "order details".into(),
          kind: CompletionKind::Table,
        },
        CatalogObject {
          schema: "public".into(),
          name: "users".into(),
          kind: CompletionKind::Function,
        },
      ],
      ..CompletionCatalog::default()
    };

    let tables = mentioned_table_refs(
      "select superusers, 'order details'; -- USERS is only mentioned in a comment",
      &catalog,
    );
    assert_eq!(
      tables,
      vec![
        TableRef { schema: "audit".into(), table: "users".into() },
        TableRef { schema: "public".into(), table: "order details".into() },
        TableRef { schema: "public".into(), table: "users".into() },
      ]
    );
  }

  #[tokio::test]
  async fn table_discovery_survives_completion_dismissal() {
    let (mut coordinator, client) = CompletionCoordinator::new();
    coordinator.catalog_loaded = true;
    coordinator.catalog.write().unwrap().objects.push(CatalogObject {
      schema: "public".into(),
      name: "users".into(),
      kind: CompletionKind::Table,
    });

    let mut completion_request = request("-- users");
    completion_request.generation = 1;
    client.command_tx.send(CompletionCommand::Request(completion_request)).unwrap();
    client.command_tx.send(CompletionCommand::Cancel { generation: 1 }).unwrap();

    for _ in 0..200 {
      coordinator.poll(Duration::from_millis(200), 2);
      if coordinator
        .missing_columns
        .contains(&TableRef { schema: "public".into(), table: "users".into() })
      {
        return;
      }
      tokio::task::yield_now().await;
    }
    panic!("dismissed completion did not queue mentioned table columns");
  }

  #[tokio::test(start_paused = true)]
  async fn coordinator_debounces_and_only_delivers_latest_generation() {
    let (mut coordinator, mut client) = CompletionCoordinator::new();
    let mut first = request("sel");
    first.generation = 1;
    let mut second = request("sele");
    second.generation = 2;
    client.command_tx.send(CompletionCommand::Request(first)).unwrap();
    coordinator.poll(Duration::from_millis(200), 10);
    client.command_tx.send(CompletionCommand::Request(second)).unwrap();
    coordinator.poll(Duration::from_millis(200), 10);
    tokio::task::yield_now().await;

    tokio::time::advance(Duration::from_millis(199)).await;
    tokio::task::yield_now().await;
    coordinator.poll(Duration::from_millis(200), 10);
    assert!(client.response_rx.try_recv().is_err());

    tokio::time::advance(Duration::from_millis(1)).await;
    for _ in 0..200 {
      tokio::task::yield_now().await;
      coordinator.poll(Duration::from_millis(200), 10);
      if let Ok(CompletionUiEvent::Response(response)) = client.response_rx.try_recv() {
        assert_eq!(response.generation, 2);
        assert!(response.candidates.is_empty());
        return;
      }
    }
    panic!("completion response was not delivered");
  }
}
