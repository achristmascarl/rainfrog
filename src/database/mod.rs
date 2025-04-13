use std::{collections::HashMap, ops::Deref, sync::Arc};

use color_eyre::eyre::Result;
use futures::stream::{BoxStream, StreamExt};
use sqlparser::{
  ast::Statement,
  dialect::{Dialect, MsSqlDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect},
  keywords,
  parser::{Parser, ParserError},
};
use sqlx::Either;
use tokio::task::JoinHandle;

use crate::cli::{Cli, Driver};

// mod mysql;
mod postgresql;
// mod sqlite;

pub use postgresql::PostgresDriver;

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
  pub statement_type: Statement,
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
    write!(f, "{:?}", self)
  }
}
impl std::error::Error for ParseError {}

pub type QueryTask = tokio::task::JoinHandle<QueryResultsWithMetadata>;

pub enum DbTaskResult {
  Finished(QueryResultsWithMetadata),
  ConfirmTx(Option<u64>, Statement),
  Pending,
  NoTask,
}

pub trait Database: Sized {
  async fn init(args: Cli) -> Result<Self>;
  fn start_query(&mut self, query: String) -> Result<()>;
  fn abort_query(&mut self) -> Result<bool>;
  async fn get_query_results(&mut self) -> Result<DbTaskResult>;

  // transactions
  async fn start_tx(&mut self, query: String) -> Result<()>;
  async fn commit_tx(&mut self) -> Result<Option<QueryResultsWithMetadata>>;
  async fn rollback_tx(&mut self) -> Result<()>;

  // preciews
  async fn load_menu(&self) -> Result<Rows>;
  fn preview_rows_query(&self, schema: &str, table: &str) -> String;
  fn preview_columns_query(&self, schema: &str, table: &str) -> String;
  fn preview_constraints_query(&self, schema: &str, table: &str) -> String;
  fn preview_indexes_query(&self, schema: &str, table: &str) -> String;
  fn preview_policies_query(&self, schema: &str, table: &str) -> String;
}

fn get_first_query(query: String, driver: Driver) -> Result<(String, Statement), ParseError> {
  let ast = Parser::parse_sql(&(get_dialect(driver)), &query);
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

pub fn get_execution_type(query: String, confirmed: bool, driver: Driver) -> Result<(ExecutionType, Statement)> {
  let first_query = get_first_query(query, driver);

  match first_query {
    Ok((_, statement)) => Ok((get_default_execution_type(statement.clone(), confirmed), statement.clone())),
    Err(e) => Err(color_eyre::eyre::Report::new(e)),
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

pub fn statement_type_string(statement: &Statement) -> String {
  format!("{:?}", statement).split('(').collect::<Vec<&str>>()[0].split('{').collect::<Vec<&str>>()[0]
    .split('[')
    .collect::<Vec<&str>>()[0]
    .trim()
    .to_string()
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

pub fn get_dialect(driver: Driver) -> impl Dialect {
  match driver {
    Driver::Postgres => PostgreSqlDialect {},
    _ => panic!("Driver not supported"),
    // "MySQL" => Arc::new(MySqlDialect {}),
    // "SQLite" => Arc::new(SQLiteDialect {}),
  }
}

pub trait HasRowsAffected {
  fn rows_affected(&self) -> u64;
}
