use async_trait::async_trait;
use color_eyre::eyre::{self, Result};
use sqlparser::{
  ast::Statement,
  dialect::{Dialect, GenericDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect},
  keywords,
  parser::{Parser, ParserError},
};
use tokio::task::JoinHandle;

use crate::cli::{Cli, Driver};

mod mysql;
mod oracle;
mod postgresql;
mod sqlite;

pub use mysql::MySqlDriver;
pub use oracle::OracleDriver;
pub use postgresql::PostgresDriver;
pub use sqlite::SqliteDriver;

#[derive(Debug, Clone)]
pub struct Header {
  pub name: String,
  pub type_name: String,
}
pub type Headers = Vec<Header>;

pub struct Value {
  pub parse_error: bool,
  pub is_null: bool,
  pub string: String,
}

#[derive(Debug, Clone)]
pub struct Rows {
  pub headers: Headers,
  pub rows: Vec<Vec<String>>,
  pub rows_affected: Option<u64>,
}

#[derive(Debug)]
pub struct QueryResultsWithMetadata {
  pub results: Result<Rows>,
  pub statement_type: Option<Statement>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionType {
  Confirm,
  Transaction,
  Normal,
}

#[derive(Debug)]
pub enum ParseError {
  MoreThanOneStatement(String),
  EmptyQuery(String),
  SqlParserError(ParserError),
}
impl std::fmt::Display for ParseError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{self:?}")
  }
}
impl std::error::Error for ParseError {}

pub type QueryTask = JoinHandle<QueryResultsWithMetadata>;

pub enum DbTaskResult {
  Finished(QueryResultsWithMetadata),
  ConfirmTx(Option<u64>, Option<Statement>),
  Pending,
  NoTask,
}

#[async_trait(?Send)]
pub trait Database {
  /// Initialize the database connection. Should handle create
  /// a pool or connection that's reused for other operations.
  /// Must be called to actually connect to the database (just
  /// calling `new()` does not connect).
  async fn init(&mut self, args: Cli) -> Result<()>;

  /// Spawns a tokio task that runs the query. The task should
  /// expect to be polled via the `get_query_results()` method.
  async fn start_query(&mut self, query: String, bypass_parser: bool) -> Result<()>;

  /// Aborts the tokio task running the active query or transaction.
  /// Some drivers also kill the process that was running the query,
  /// so that the query does not continue running in the background.
  /// This behavior needs to be implemented by the driver.
  async fn abort_query(&mut self) -> Result<bool>;

  /// Polls the tokio task for the active query or transaction if
  /// it exists. Returns `DbTaskResult::NoTask` if no task is running,
  /// `DbTaskResult::Pending` if the task is still running.
  async fn get_query_results(&mut self) -> Result<DbTaskResult>;

  /// Spawns a tokio task that runs the query in a transaction.
  /// The task should also expect to be polled via the `get_query_results()`
  /// method.
  async fn start_tx(&mut self, query: String) -> Result<()>;

  /// Commits the pending transaction and returns the results.
  /// Should do nothing or fail gracefully if no transaction is pending.
  async fn commit_tx(&mut self) -> Result<Option<QueryResultsWithMetadata>>;

  /// Rolls back the pending transaction. Should do nothing or fail gracefully
  /// if no transaction is pending.
  async fn rollback_tx(&mut self) -> Result<()>;

  /// Returns rows representing the database menu. The menu component
  /// expects each row to be combination of schema and table name.
  async fn load_menu(&self) -> Result<Rows>;

  /// Returns a query that can be used to preview the rows in a table.
  fn preview_rows_query(&self, schema: &str, table: &str) -> String;

  /// Returns a query that can be used to preview the columns in a table.
  fn preview_columns_query(&self, schema: &str, table: &str) -> String;

  /// Returns a query that can be used to preview the constraints in a table.
  fn preview_constraints_query(&self, schema: &str, table: &str) -> String;

  /// Returns a query that can be used to preview the indexes in a table.
  fn preview_indexes_query(&self, schema: &str, table: &str) -> String;

  /// Returns a query that can be used to preview the policies in a table.
  fn preview_policies_query(&self, schema: &str, table: &str) -> String;
}

fn get_first_query(query: String, driver: Driver) -> Result<(String, Statement), ParseError> {
  let ast = Parser::parse_sql(&*get_dialect(driver), &query);
  match ast {
    Ok(ast) if ast.len() > 1 => {
      Err(ParseError::MoreThanOneStatement("Only one statement allowed per query".to_owned()))
    },
    Ok(ast) if ast.is_empty() => Err(ParseError::EmptyQuery("Parsed query is empty".to_owned())),
    Ok(ast) => {
      let statement = ast[0].clone();
      Ok((statement.to_string(), statement))
    },
    Err(e) => Err(ParseError::SqlParserError(e)),
  }
}

pub fn get_execution_type(
  query: String,
  confirmed: bool,
  driver: Driver,
) -> Result<(ExecutionType, Option<Statement>)> {
  let first_query = get_first_query(query, driver);

  match first_query {
    Ok((_, statement)) => Ok((get_default_execution_type(statement.clone(), confirmed), Some(statement.clone()))),
    Err(e) => Err(eyre::Report::new(e)),
  }
}

fn get_default_execution_type(statement: Statement, confirmed: bool) -> ExecutionType {
  if confirmed {
    return ExecutionType::Normal;
  }
  match statement {
    Statement::AlterIndex { .. }
    | Statement::AlterView { .. }
    | Statement::AlterRole { .. }
    | Statement::AlterTable { .. }
    | Statement::Drop { .. }
    | Statement::Truncate { .. } => ExecutionType::Confirm,
    Statement::Delete(_) | Statement::Update { .. } => ExecutionType::Transaction,
    Statement::Explain { statement, analyze, .. }
      if analyze
        && matches!(
          statement.as_ref(),
          Statement::AlterIndex { .. }
            | Statement::AlterView { .. }
            | Statement::AlterRole { .. }
            | Statement::AlterTable { .. }
            | Statement::Drop { .. }
            | Statement::Truncate { .. },
        ) =>
    {
      ExecutionType::Confirm
    },
    Statement::Explain { statement, analyze, .. }
      if analyze && matches!(statement.as_ref(), Statement::Delete(_) | Statement::Update { .. }) =>
    {
      ExecutionType::Transaction
    },
    Statement::Explain { .. } => ExecutionType::Normal,
    _ => ExecutionType::Normal,
  }
}

pub fn statement_type_string(statement: Option<Statement>) -> String {
  match statement {
    Some(stmt) => format!("{stmt:?}").split('(').collect::<Vec<&str>>()[0].split('{').collect::<Vec<&str>>()[0]
      .split('[')
      .collect::<Vec<&str>>()[0]
      .trim()
      .to_string(),
    None => "UNKNOWN".to_string(),
  }
}

pub fn vec_to_string<T: std::string::ToString>(vec: Vec<T>) -> String {
  let mut content = String::new();
  for (i, elem) in vec.iter().enumerate() {
    content.push_str(&elem.to_string());
    if i != vec.len() - 1 {
      content.push_str(", ");
    }
  }
  "{ ".to_owned() + &*content + &*" }".to_owned()
}

pub fn header_to_vec(headers: &Headers) -> Vec<String> {
  headers.iter().map(|h| h.name.to_string()).collect()
}

pub fn get_keywords() -> Vec<String> {
  keywords::ALL_KEYWORDS.iter().map(|k| k.to_string()).collect()
}

pub fn get_dialect(driver: Driver) -> Box<dyn Dialect + Send + Sync> {
  match driver {
    Driver::Postgres => Box::new(PostgreSqlDialect {}),
    Driver::MySql => Box::new(MySqlDialect {}),
    Driver::Sqlite => Box::new(SQLiteDialect {}),
    Driver::Oracle => Box::new(GenericDialect {}),
  }
}

pub trait HasRowsAffected {
  fn rows_affected(&self) -> u64;
}
