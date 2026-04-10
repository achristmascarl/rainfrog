use async_trait::async_trait;
use color_eyre::eyre::Result;
#[cfg(feature = "duckdb")]
use sqlparser::dialect::DuckDbDialect;

use sqlparser::{
  ast::{Query, SetExpr, Statement},
  dialect::{Dialect, GenericDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect},
  keywords,
  parser::{Parser, ParserError},
};
use tokio::task::JoinHandle;

use crate::cli::{Cli, Driver};

#[cfg(feature = "duckdb")]
mod duckdb;
mod mysql;
mod oracle;
mod postgresql;
mod sqlite;

#[cfg(feature = "duckdb")]
pub use duckdb::DuckDbDriver;
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
  /// calling `new()` does not connect). Returns the default title
  /// for the terminal tab.
  async fn init(&mut self, args: Cli) -> Result<String>;

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
  /// expects each row to be combination of schema, object name, and kind.
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

  /// Returns a query that can be used to preview the definition of a view.
  fn preview_view_definition_query(&self, schema: &str, view: &str, materialized: bool) -> String;

  /// Returns a query that can be used to preview the definition of a function.
  fn preview_function_definition_query(&self, _schema: &str, _function: &str) -> String {
    "select 'Function definition preview is not available for this driver' as message".to_owned()
  }
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
  let (_, statement) = get_first_query(query, driver)?;
  let default_execution_info = get_default_execution_info(&statement, confirmed);

  match driver {
    #[cfg(feature = "duckdb")]
    Driver::DuckDb => match default_execution_info.execution_type {
      ExecutionType::Normal => Ok((ExecutionType::Normal, Some(default_execution_info.statement))),
      ExecutionType::Confirm => {
        Ok((ExecutionType::Confirm, Some(default_execution_info.statement)))
      },
      ExecutionType::Transaction => {
        Ok((ExecutionType::Confirm, Some(default_execution_info.statement)))
      }, // don't allow auto-transactions
    },
    _ => Ok((default_execution_info.execution_type, Some(default_execution_info.statement))),
  }
}

#[derive(Debug)]
struct ExecutionTypeInfo {
  execution_type: ExecutionType,
  statement: Statement,
}

fn get_default_execution_info(statement: &Statement, confirmed: bool) -> ExecutionTypeInfo {
  if confirmed {
    return ExecutionTypeInfo {
      execution_type: ExecutionType::Normal,
      statement: statement.clone(),
    };
  }

  get_statement_execution_type(statement)
}

fn get_statement_for_execution_type(statement: &Statement) -> Statement {
  get_statement_execution_type(statement).statement
}

fn get_statement_execution_type(statement: &Statement) -> ExecutionTypeInfo {
  match statement {
    Statement::AlterIndex { .. }
    | Statement::AlterView { .. }
    | Statement::AlterRole { .. }
    | Statement::AlterTable { .. }
    | Statement::Drop { .. }
    | Statement::Truncate { .. } => {
      ExecutionTypeInfo { execution_type: ExecutionType::Confirm, statement: statement.clone() }
    },
    Statement::Delete(_) | Statement::Update { .. } => {
      ExecutionTypeInfo { execution_type: ExecutionType::Transaction, statement: statement.clone() }
    },
    Statement::Query(query) => get_query_execution_type(query).unwrap_or_else(|| {
      ExecutionTypeInfo { execution_type: ExecutionType::Normal, statement: statement.clone() }
    }),
    Statement::Explain { statement: explained_statement, analyze, .. } if *analyze => {
      let info = get_statement_execution_type(explained_statement);
      if info.execution_type == ExecutionType::Normal {
        ExecutionTypeInfo { execution_type: ExecutionType::Normal, statement: statement.clone() }
      } else {
        info
      }
    },
    Statement::Explain { .. } => {
      ExecutionTypeInfo { execution_type: ExecutionType::Normal, statement: statement.clone() }
    },
    _ => ExecutionTypeInfo { execution_type: ExecutionType::Normal, statement: statement.clone() },
  }
}

fn get_query_execution_type(query: &Query) -> Option<ExecutionTypeInfo> {
  let cte_execution_type = query
    .with
    .as_ref()
    .map(|with| {
      with
        .cte_tables
        .iter()
        .map(|cte| get_query_execution_type(&cte.query))
        .fold(None, max_execution_type)
    })
    .unwrap_or(None);

  max_execution_type(cte_execution_type, get_set_expr_execution_type(&query.body))
}

fn get_set_expr_execution_type(set_expr: &SetExpr) -> Option<ExecutionTypeInfo> {
  match set_expr {
    SetExpr::Query(query) => get_query_execution_type(query),
    SetExpr::SetOperation { left, right, .. } => {
      max_execution_type(get_set_expr_execution_type(left), get_set_expr_execution_type(right))
    },
    SetExpr::Insert(statement)
    | SetExpr::Update(statement)
    | SetExpr::Delete(statement)
    | SetExpr::Merge(statement) => Some(get_statement_execution_type(statement)),
    _ => None,
  }
}

fn max_execution_type(
  left: Option<ExecutionTypeInfo>,
  right: Option<ExecutionTypeInfo>,
) -> Option<ExecutionTypeInfo> {
  match (left, right) {
    (Some(left), Some(right)) => Some(max_execution_type_info(left, right)),
    (Some(left), None) => Some(left),
    (None, Some(right)) => Some(right),
    (None, None) => None,
  }
}

fn max_execution_type_info(left: ExecutionTypeInfo, right: ExecutionTypeInfo) -> ExecutionTypeInfo {
  match (&left.execution_type, &right.execution_type) {
    (ExecutionType::Confirm, _) | (_, ExecutionType::Normal) => left,
    (ExecutionType::Normal, _) | (_, ExecutionType::Confirm) => right,
    (ExecutionType::Transaction, ExecutionType::Transaction) => left,
  }
}

pub fn statement_type_string(statement: Option<Statement>) -> String {
  match statement {
    Some(stmt) => {
      format!("{stmt:?}").split('(').collect::<Vec<&str>>()[0].split('{').collect::<Vec<&str>>()[0]
        .split('[')
        .collect::<Vec<&str>>()[0]
        .trim()
        .to_string()
    },
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
    #[cfg(feature = "duckdb")]
    Driver::DuckDb => Box::new(DuckDbDialect {}),
  }
}

pub trait HasRowsAffected {
  fn rows_affected(&self) -> u64;
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn cte_wrapped_writes_use_write_execution_type() {
    let test_cases = vec![
      (
        "WITH rows AS (SELECT id FROM users WHERE id = 1) \
         UPDATE users SET name = 'Jane' WHERE id IN (SELECT id FROM rows)",
        ExecutionType::Transaction,
      ),
      (
        "WITH rows AS (SELECT id FROM users WHERE id = 1) \
         DELETE FROM users WHERE id IN (SELECT id FROM rows)",
        ExecutionType::Transaction,
      ),
      (
        "WITH rows AS (SELECT id FROM users WHERE id = 1) SELECT * FROM rows",
        ExecutionType::Normal,
      ),
    ];

    for (query, expected) in test_cases {
      let (execution_type, statement) =
        get_execution_type(query.to_string(), false, Driver::Postgres).unwrap();

      assert_eq!(execution_type, expected, "Failed for query: {query}");

      match expected {
        ExecutionType::Transaction => {
          assert!(
            matches!(statement, Some(Statement::Update { .. }) | Some(Statement::Delete(_))),
            "Expected write statement for query: {query}, got {statement:?}"
          );
        },
        ExecutionType::Normal => {
          assert!(
            matches!(statement, Some(Statement::Query(_))),
            "Expected query statement for query: {query}, got {statement:?}"
          );
        },
        ExecutionType::Confirm => {},
      }
    }
  }

  #[test]
  fn writes_inside_ctes_use_write_execution_type() {
    let query = "WITH deleted AS (DELETE FROM users WHERE id = 1 RETURNING id) \
                 SELECT * FROM deleted";

    let (execution_type, statement) =
      get_execution_type(query.to_string(), false, Driver::Postgres).unwrap();

    assert_eq!(execution_type, ExecutionType::Transaction);
    assert!(matches!(statement, Some(Statement::Delete(_))));
  }

  #[test]
  fn explain_analyze_cte_wrapped_writes_use_write_execution_type() {
    let query = "EXPLAIN ANALYZE WITH rows AS (SELECT id FROM users WHERE id = 1) \
                 UPDATE users SET name = 'Jane' WHERE id IN (SELECT id FROM rows)";

    let (execution_type, statement) =
      get_execution_type(query.to_string(), false, Driver::Postgres).unwrap();

    assert_eq!(execution_type, ExecutionType::Transaction);
    assert!(matches!(statement, Some(Statement::Update { .. })));
  }

  #[test]
  fn explain_without_analyze_cte_wrapped_writes_are_normal() {
    let query = "EXPLAIN WITH rows AS (SELECT id FROM users WHERE id = 1) \
                 UPDATE users SET name = 'Jane' WHERE id IN (SELECT id FROM rows)";

    assert_eq!(
      get_execution_type(query.to_string(), false, Driver::Postgres).unwrap().0,
      ExecutionType::Normal
    );
  }
}
