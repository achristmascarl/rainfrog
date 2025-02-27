use std::{collections::HashMap, sync::Arc};

use futures::stream::{BoxStream, StreamExt};
use sqlparser::{
  ast::Statement,
  dialect::{Dialect, MsSqlDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect},
  keywords,
  parser::{Parser, ParserError},
};
use sqlx::{
  mysql::{MySql, MySqlColumn, MySqlQueryResult, MySqlRow},
  pool::PoolOptions,
  postgres::{PgColumn, PgQueryResult, PgRow, Postgres},
  sqlite::{Sqlite, SqliteColumn, SqliteQueryResult, SqliteRow},
  Column, Connection, Database, Either, Error, Executor, Pool, Row, Transaction,
};

use crate::cli::Cli;

mod mysql;
mod postgresql;
mod sqlite;

#[derive(Debug, Clone)]
pub struct Header {
  pub name: String,
  pub type_name: String,
}

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
pub type Headers = Vec<Header>;
pub type DbPool<DB> = Pool<DB>;
pub type DbError = Either<Error, ParserError>;

#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionType {
  Confirm,
  Transaction,
  Normal,
}

pub trait HasRowsAffected {
  fn rows_affected(&self) -> u64;
}

pub trait DatabaseQueries {
  fn preview_tables_query() -> String;
  fn preview_rows_query(schema: &str, table: &str) -> String;
  fn preview_columns_query(schema: &str, table: &str) -> String;
  fn preview_constraints_query(schema: &str, table: &str) -> String;
  fn preview_indexes_query(schema: &str, table: &str) -> String;
  fn preview_policies_query(schema: &str, table: &str) -> String;
}

pub trait ValueParser: Database {
  fn parse_value(row: &Self::Row, col: &Self::Column) -> Option<Value>;
}

pub trait BuildConnectionOptions: Database {
  fn build_connection_opts(args: Cli) -> color_eyre::eyre::Result<<Self::Connection as Connection>::Options>;
}

pub async fn init_pool<DB: Database>(opts: <DB::Connection as Connection>::Options) -> Result<Pool<DB>, Error> {
  PoolOptions::new().max_connections(3).connect_with(opts).await
}

// since it's possible for raw_sql to execute multiple queries in a single string,
// we only execute the first one and then drop the rest.
pub async fn query<DB>(query: String, dialect: &(dyn Dialect + Sync), pool: &Pool<DB>) -> Result<Rows, DbError>
where
  DB: Database + ValueParser,
  DB::QueryResult: HasRowsAffected,
  for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
{
  let first_query = get_first_query(query, dialect);
  match first_query {
    Ok((first_query, _)) => {
      let stream = sqlx::raw_sql(&first_query).fetch_many(pool);
      query_stream::<DB>(stream).await
    },
    Err(e) => Err(e),
  }
}

#[allow(clippy::type_complexity)]
pub async fn query_stream<DB>(
  mut stream: BoxStream<'_, Result<Either<DB::QueryResult, DB::Row>, Error>>,
) -> Result<Rows, DbError>
where
  DB: Database + ValueParser,
  DB::QueryResult: HasRowsAffected,
{
  let mut query_rows = vec![];
  let mut query_rows_affected: Option<u64> = None;
  let mut headers: Headers = vec![];
  // I change the implementation of the while loop here as the original one times out mysql connection
  while let Some(item) = stream.next().await {
    match item {
      Ok(Either::Left(result)) => {
        // For non-SELECT queries
        query_rows_affected = Some(result.rows_affected());
      },
      Ok(Either::Right(row)) => {
        // For SELECT queries
        query_rows.push(row_to_vec::<DB>(&row));
        if headers.is_empty() {
          headers = get_headers::<DB>(&row);
        }
      },
      Err(e) => return Err(Either::Left(e)),
    }
  }
  Ok(Rows { rows_affected: query_rows_affected, headers, rows: query_rows })
}

pub async fn query_with_tx<'a, DB>(
  mut tx: Transaction<'static, DB>,
  dialect: &(dyn Dialect + Sync),
  query: String,
) -> (Result<Either<u64, Rows>, DbError>, Transaction<'static, DB>)
where
  DB: Database + ValueParser,
  DB::QueryResult: HasRowsAffected,
  for<'c> <DB as sqlx::Database>::Arguments<'c>: sqlx::IntoArguments<'c, DB>,
  for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
{
  let first_query = get_first_query(query, dialect);
  match first_query {
    Ok((first_query, statement_type)) => {
      match statement_type {
        Statement::Explain { .. } => {
          let stream = sqlx::raw_sql(&first_query).fetch_many(&mut *tx);
          let result = query_stream::<DB>(stream).await;
          match result {
            Ok(result) => (Ok(Either::Right(result)), tx),
            Err(e) => (Err(e), tx),
          }
        },
        _ => {
          let result = sqlx::query(&first_query).execute(&mut *tx).await;
          match result {
            Ok(result) => (Ok(Either::Left(result.rows_affected())), tx),
            Err(e) => (Err(DbError::Left(e)), tx),
          }
        },
      }
    },
    Err(e) => (Err(e), tx),
  }
}

pub fn get_first_query(query: String, dialect: &dyn Dialect) -> Result<(String, Statement), DbError> {
  let ast = Parser::parse_sql(dialect, &query);
  match ast {
    Ok(ast) if ast.len() > 1 => {
      Err(Either::Right(ParserError::ParserError("Only one statement allowed per query".to_owned())))
    },
    Ok(ast) if ast.is_empty() => Err(Either::Right(ParserError::ParserError("Parsed query is empty".to_owned()))),
    Ok(ast) => {
      let statement = ast[0].clone();
      Ok((statement.to_string(), statement))
    },
    Err(e) => Err(Either::Right(e)),
  }
}

pub fn statement_type_string(statement: &Statement) -> String {
  format!("{:?}", statement).split('(').collect::<Vec<&str>>()[0].split('{').collect::<Vec<&str>>()[0]
    .split('[')
    .collect::<Vec<&str>>()[0]
    .trim()
    .to_string()
}

pub fn get_execution_type(statement: Statement, confirmed: bool) -> ExecutionType {
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

pub fn get_headers<DB: Database + ValueParser>(row: &DB::Row) -> Headers {
  row
    .columns()
    .iter()
    .map(|col| Header { name: col.name().to_string(), type_name: col.type_info().to_string() })
    .collect()
}

pub fn row_to_json<DB: Database + ValueParser>(row: &DB::Row) -> HashMap<String, String> {
  let mut result = HashMap::new();
  for col in row.columns() {
    let value = match DB::parse_value(row, col) {
      Some(v) => v.string,
      _ => "[ unsupported ]".to_string(),
    };
    result.insert(col.name().to_string(), value);
  }

  result
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

pub fn row_to_vec<DB: Database + ValueParser>(row: &DB::Row) -> Vec<String> {
  row.columns().iter().map(|col| DB::parse_value(row, col).unwrap().string).collect()
}

pub fn header_to_vec(headers: &Headers) -> Vec<String> {
  headers.iter().map(|h| h.name.to_string()).collect()
}

pub fn get_keywords() -> Vec<String> {
  keywords::ALL_KEYWORDS.iter().map(|k| k.to_string()).collect()
}

pub fn get_dialect(db_type: &str) -> Arc<dyn Dialect + Send + Sync> {
  match db_type {
    "PostgreSQL" => Arc::new(PostgreSqlDialect {}),
    "MySQL" => Arc::new(MySqlDialect {}),
    "SQLite" => Arc::new(SQLiteDialect {}),
    x => panic!("Unsupported database type: {}", x),
  }
}
