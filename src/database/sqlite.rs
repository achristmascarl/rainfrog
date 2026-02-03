use std::{
  fmt::Write,
  io::{self, Write as _},
  str::FromStr,
  string::String,
  sync::Arc,
};

use async_trait::async_trait;
use color_eyre::eyre::{self, Result};
use futures::stream::StreamExt;
use sqlparser::ast::Statement;
use sqlx::{
  Column, Either, Row, ValueRef,
  sqlite::{Sqlite, SqliteConnectOptions, SqlitePoolOptions},
  types::uuid,
};

use super::{Database, DbTaskResult, Driver, Header, Headers, QueryResultsWithMetadata, QueryTask, Rows, Value};

type SqliteTransaction<'a> = sqlx::Transaction<'a, Sqlite>;
type TransactionTask<'a> = tokio::task::JoinHandle<(QueryResultsWithMetadata, SqliteTransaction<'a>)>;
enum SqliteTask<'a> {
  Query(QueryTask),
  TxStart(TransactionTask<'a>),
  TxPending(Box<(SqliteTransaction<'a>, QueryResultsWithMetadata)>),
}

#[derive(Default)]
pub struct SqliteDriver<'a> {
  pool: Option<Arc<sqlx::Pool<Sqlite>>>,
  task: Option<SqliteTask<'a>>,
}

#[async_trait(?Send)]
impl Database for SqliteDriver<'_> {
  async fn init(&mut self, args: crate::cli::Cli) -> Result<()> {
    let opts = super::sqlite::SqliteDriver::<'_>::build_connection_opts(args)?;
    let pool = Arc::new(SqlitePoolOptions::new().max_connections(3).connect_with(opts).await?);
    self.pool = Some(pool);
    Ok(())
  }

  // since it's possible for raw_sql to execute multiple queries in a single string,
  // we only execute the first one and then drop the rest.
  async fn start_query(&mut self, query: String, bypass_parser: bool) -> Result<()> {
    let (first_query, statement_type) = match bypass_parser {
      true => (query, None),
      false => {
        let (first, stmt) = super::get_first_query(query, Driver::Sqlite)?;
        (first, Some(stmt))
      },
    };
    let pool = self.pool.clone().unwrap();
    self.task = Some(SqliteTask::Query(tokio::spawn(async move {
      let results = query_with_pool(pool, first_query.clone()).await;
      match results {
        Ok(ref rows) => {
          log::info!("{:?} rows, {:?} affected", rows.rows.len(), rows.rows_affected);
        },
        Err(ref e) => {
          log::error!("{e:?}");
        },
      };
      QueryResultsWithMetadata { results, statement_type: statement_type.clone() }
    })));
    Ok(())
  }

  async fn abort_query(&mut self) -> Result<bool> {
    match self.task.take() {
      Some(task) => {
        match task {
          SqliteTask::Query(handle) => handle.abort(),
          SqliteTask::TxStart(handle) => handle.abort(),
          _ => {},
        };
        Ok(true)
      },
      _ => Ok(false),
    }
  }

  async fn get_query_results(&mut self) -> Result<DbTaskResult> {
    let (task_result, next_task) = match self.task.take() {
      None => (DbTaskResult::NoTask, None),
      Some(SqliteTask::Query(handle)) => {
        if !handle.is_finished() {
          (DbTaskResult::Pending, Some(SqliteTask::Query(handle)))
        } else {
          let result = handle.await?;
          (DbTaskResult::Finished(result), None)
        }
      },
      Some(SqliteTask::TxStart(handle)) => {
        if !handle.is_finished() {
          (DbTaskResult::Pending, Some(SqliteTask::TxStart(handle)))
        } else {
          let (result, tx) = handle.await?;
          let rows_affected = match &result.results {
            Ok(rows) => rows.rows_affected,
            _ => None,
          };
          match result {
            // if tx failed to start, return the error immediately
            QueryResultsWithMetadata { results: Err(e), statement_type } => {
              log::error!("Transaction didn't start: {e:?}");
              (DbTaskResult::Finished(QueryResultsWithMetadata { results: Err(e), statement_type }), None)
            },
            _ => (
              DbTaskResult::ConfirmTx(rows_affected, result.statement_type.clone()),
              Some(SqliteTask::TxPending(Box::new((tx, result)))),
            ),
          }
        }
      },
      Some(SqliteTask::TxPending(b)) => (DbTaskResult::Pending, Some(SqliteTask::TxPending(b))),
    };
    self.task = next_task;
    Ok(task_result)
  }

  async fn start_tx(&mut self, query: String) -> Result<()> {
    let (first_query, statement_type) = super::get_first_query(query, Driver::Sqlite)?;
    let tx = self.pool.as_mut().unwrap().begin().await?;
    self.task = Some(SqliteTask::TxStart(tokio::spawn(async move {
      let (results, tx) = query_with_tx(tx, &first_query).await;
      match results {
        Ok(Either::Left(rows_affected)) => {
          log::info!("{rows_affected:?} rows affected");
          (
            QueryResultsWithMetadata {
              results: Ok(Rows { headers: vec![], rows: vec![], rows_affected: Some(rows_affected) }),
              statement_type: Some(statement_type),
            },
            tx,
          )
        },
        Ok(Either::Right(rows)) => {
          log::info!("{:?} rows affected", rows.rows_affected);
          (QueryResultsWithMetadata { results: Ok(rows), statement_type: Some(statement_type) }, tx)
        },
        Err(e) => {
          log::error!("{e:?}");
          (QueryResultsWithMetadata { results: Err(e), statement_type: Some(statement_type) }, tx)
        },
      }
    })));
    Ok(())
  }

  async fn commit_tx(&mut self) -> Result<Option<QueryResultsWithMetadata>> {
    if !matches!(self.task, Some(SqliteTask::TxPending(_))) {
      Ok(None)
    } else {
      match self.task.take() {
        Some(SqliteTask::TxPending(b)) => {
          b.0.commit().await?;
          Ok(Some(b.1))
        },
        _ => Ok(None),
      }
    }
  }

  async fn rollback_tx(&mut self) -> Result<()> {
    if let Some(SqliteTask::TxPending(b)) = self.task.take() {
      b.0.rollback().await?;
    }
    Ok(())
  }

  async fn load_menu(&self) -> Result<Rows> {
    query_with_pool(
      self.pool.clone().unwrap(),
      "select '' as table_schema,
        name as table_name,
        case
          when type = 'table' then 'table'
          when type = 'view' then 'view'
          else 'table'
        end as object_kind
      from sqlite_master
      where type in ('table', 'view')
      and name not like 'sqlite_%'
      order by object_kind, name asc"
        .to_owned(),
    )
    .await
  }

  fn preview_rows_query(&self, schema: &str, table: &str) -> String {
    format!("select * from \"{table}\" limit 100")
  }

  fn preview_columns_query(&self, schema: &str, table: &str) -> String {
    format!("pragma table_info(\"{table}\")")
  }

  fn preview_constraints_query(&self, schema: &str, table: &str) -> String {
    format!("pragma foreign_key_list(\"{table}\")")
  }

  fn preview_indexes_query(&self, schema: &str, table: &str) -> String {
    format!("pragma index_list(\"{table}\")")
  }

  fn preview_policies_query(&self, schema: &str, table: &str) -> String {
    "select 'SQLite does not support row-level security policies' as message".to_owned()
  }

  fn preview_view_definition_query(&self, schema: &str, view: &str, materialized: bool) -> String {
    if materialized {
      return "select 'SQLite does not support materialized views' as message".to_owned();
    }
    format!("select sql as definition from sqlite_master where type = 'view' and name = '{view}'")
  }
}

impl SqliteDriver<'_> {
  pub fn new() -> Self {
    Self { pool: None, task: None }
  }

  fn build_connection_opts(
    args: crate::cli::Cli,
  ) -> Result<<<sqlx::Sqlite as sqlx::Database>::Connection as sqlx::Connection>::Options> {
    match args.connection_url {
      Some(url) => Ok(SqliteConnectOptions::from_str(url.trim().trim_start_matches("jdbc:"))?),
      None => {
        let filename = if let Some(database) = args.database {
          database
        } else {
          let mut database = String::new();
          print!("database file path (or ':memory:'): ");
          io::stdout().flush()?;
          io::stdin().read_line(&mut database)?;
          let database = database.trim().to_string();
          if database.is_empty() {
            return Err(eyre::Report::msg("Database file path is required"));
          }
          database
        };

        let opts = SqliteConnectOptions::new().filename(&filename);
        Ok(opts)
      },
    }
  }
}

async fn query_with_pool(pool: Arc<sqlx::Pool<Sqlite>>, query: String) -> Result<Rows> {
  query_with_stream(&*pool.clone(), &query).await
}

async fn query_with_stream<'a, E>(e: E, query: &'a str) -> Result<Rows>
where
  E: sqlx::Executor<'a, Database = sqlx::Sqlite>,
{
  let mut stream = sqlx::raw_sql(query).fetch_many(e);
  let mut query_rows = vec![];
  let mut query_rows_affected: Option<u64> = None;
  let mut headers: Headers = vec![];
  while let Some(item) = stream.next().await {
    match item {
      Ok(Either::Left(result)) => {
        // For non-SELECT queries
        query_rows_affected = Some(result.rows_affected());
      },
      Ok(Either::Right(row)) => {
        // For SELECT queries
        query_rows.push(row_to_vec(&row));
        if headers.is_empty() {
          headers = get_headers(&row);
        }
      },
      Err(e) => return Err(eyre::Report::new(e)),
    }
  }
  Ok(Rows { rows_affected: query_rows_affected, headers, rows: query_rows })
}

async fn query_with_tx<'a>(
  mut tx: SqliteTransaction<'static>,
  query: &str,
) -> (Result<Either<u64, Rows>>, SqliteTransaction<'static>)
where
  for<'c> <sqlx::Sqlite as sqlx::Database>::Arguments<'c>: sqlx::IntoArguments<'c, sqlx::Sqlite>,
  for<'c> &'c mut <sqlx::Sqlite as sqlx::Database>::Connection: sqlx::Executor<'c, Database = sqlx::Sqlite>,
{
  let first_query = super::get_first_query(query.to_string(), Driver::Sqlite);
  match first_query {
    Ok((first_query, statement_type)) => match statement_type {
      Statement::Explain { .. } => {
        let result = query_with_stream(&mut *tx, &first_query).await;
        match result {
          Ok(result) => (Ok(Either::Right(result)), tx),
          Err(e) => (Err(e), tx),
        }
      },
      _ => {
        let result = sqlx::query(&first_query).execute(&mut *tx).await;
        match result {
          Ok(result) => (Ok(Either::Left(result.rows_affected())), tx),
          Err(e) => (Err(e.into()), tx),
        }
      },
    },
    Err(e) => (Err(eyre::Report::new(e)), tx),
  }
}

fn get_headers(row: &<sqlx::Sqlite as sqlx::Database>::Row) -> Headers {
  row
    .columns()
    .iter()
    .map(|col| Header { name: col.name().to_string(), type_name: col.type_info().to_string() })
    .collect()
}

fn row_to_vec(row: &<sqlx::Sqlite as sqlx::Database>::Row) -> Vec<String> {
  row.columns().iter().map(|col| parse_value(row, col).unwrap().string).collect()
}

// parsed based on https://docs.rs/sqlx/latest/sqlx/sqlite/types/index.html
fn parse_value(row: &<Sqlite as sqlx::Database>::Row, col: &<Sqlite as sqlx::Database>::Column) -> Option<Value> {
  let col_type = col.type_info().to_string();
  if row.try_get_raw(col.ordinal()).is_ok_and(|v| v.is_null()) {
    return Some(Value { parse_error: false, string: "NULL".to_string(), is_null: true });
  }
  match col_type.to_uppercase().as_str() {
    "BOOLEAN" => Some(row.try_get::<bool, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "INTEGER" | "INT4" | "INT8" | "BIGINT" => Some(row.try_get::<i64, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "REAL" => Some(row.try_get::<f64, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "TEXT" => {
      // Try parsing as different types that might be stored as TEXT
      match row.try_get::<chrono::NaiveDateTime, _>(col.ordinal()) {
        Ok(dt) => Some(Value { parse_error: false, string: dt.to_string(), is_null: false }),
        _ => match row.try_get::<chrono::DateTime<chrono::Utc>, _>(col.ordinal()) {
          Ok(dt) => Some(Value { parse_error: false, string: dt.to_string(), is_null: false }),
          _ => match row.try_get::<chrono::NaiveDate, _>(col.ordinal()) {
            Ok(date) => Some(Value { parse_error: false, string: date.to_string(), is_null: false }),
            _ => match row.try_get::<chrono::NaiveTime, _>(col.ordinal()) {
              Ok(time) => Some(Value { parse_error: false, string: time.to_string(), is_null: false }),
              _ => match row.try_get::<uuid::Uuid, _>(col.ordinal()) {
                Ok(uuid) => Some(Value { parse_error: false, string: uuid.to_string(), is_null: false }),
                _ => match row.try_get::<serde_json::Value, _>(col.ordinal()) {
                  Ok(json) => Some(Value { parse_error: false, string: json.to_string(), is_null: false }),
                  _ => match row.try_get::<String, _>(col.ordinal()) {
                    Ok(string) => Some(Value { parse_error: false, string, is_null: false }),
                    _ => Some(Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false }),
                  },
                },
              },
            },
          },
        },
      }
    },
    "BLOB" => Some(row.try_get::<Vec<u8>, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| {
        if let Ok(s) = String::from_utf8(received.clone()) {
          Value { parse_error: false, string: s, is_null: false }
        } else {
          Value {
            parse_error: false,
            string: received.iter().fold(String::new(), |mut output, b| {
              let _ = write!(output, "{b:02X}");
              output
            }),
            is_null: false,
          }
        }
      },
    )),
    "DATETIME" => {
      // Similar to TEXT, but we'll try timestamp first
      match row.try_get::<i64, _>(col.ordinal()) {
        Ok(dt) => Some(chrono::DateTime::from_timestamp(dt, 0).map_or(
          Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
          |received| Value { parse_error: false, string: received.to_string(), is_null: false },
        )),
        _ => match row.try_get::<chrono::NaiveDateTime, _>(col.ordinal()) {
          Ok(dt) => Some(Value { parse_error: true, string: dt.to_string(), is_null: false }),
          _ => match row.try_get::<chrono::DateTime<chrono::Utc>, _>(col.ordinal()) {
            Ok(dt) => Some(Value { parse_error: true, string: dt.to_string(), is_null: false }),
            _ => Some(row.try_get::<String, usize>(col.ordinal()).map_or(
              Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
              |received| Value { parse_error: false, string: received.to_string(), is_null: false },
            )),
          },
        },
      }
    },
    "DATE" => match row.try_get::<chrono::NaiveDate, _>(col.ordinal()) {
      Ok(date) => Some(Value { parse_error: true, string: date.to_string(), is_null: false }),
      _ => Some(row.try_get::<String, usize>(col.ordinal()).map_or(
        Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
        |received| Value { parse_error: false, string: received.to_string(), is_null: false },
      )),
    },
    "TIME" => match row.try_get::<chrono::NaiveTime, _>(col.ordinal()) {
      Ok(time) => Some(Value { parse_error: true, string: time.to_string(), is_null: false }),
      _ => Some(row.try_get::<String, usize>(col.ordinal()).map_or(
        Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
        |received| Value { parse_error: false, string: received.to_string(), is_null: false },
      )),
    },
    _ => {
      // For any other types, try to cast to string
      Some(row.try_get_unchecked::<String, usize>(col.ordinal()).map_or(
        Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
        |received| Value { parse_error: false, string: received.to_string(), is_null: false },
      ))
    },
  }
}

#[cfg(test)]
mod tests {
  use sqlparser::{ast::Statement, dialect::SQLiteDialect, parser::ParserError};

  use super::*;
  use crate::database::{ExecutionType, ParseError, get_execution_type, get_first_query};

  #[test]
  fn test_get_first_query() {
    type TestCase = (&'static str, Result<(String, Box<dyn Fn(Statement) -> bool>), ParseError>);

    let test_cases: Vec<TestCase> = vec![
      // single query
      ("SELECT * FROM users;", Ok(("SELECT * FROM users".to_string(), Box::new(|s| matches!(s, Statement::Query(_)))))),
      // multiple queries
      (
        "SELECT * FROM users; DELETE FROM posts;",
        Err(ParseError::MoreThanOneStatement("Only one statement allowed per query".to_owned())),
      ),
      // empty query
      ("", Err(ParseError::EmptyQuery("Parsed query is empty".to_owned()))),
      // syntax error
      (
        "SELEC * FORM users;",
        Err(ParseError::SqlParserError(ParserError::ParserError(
          "Expected: an SQL statement, found: SELEC at Line: 1, Column: 1".to_owned(),
        ))),
      ),
      // lowercase
      (
        "select * from `users`",
        Ok(("SELECT * FROM `users`".to_owned(), Box::new(|s| matches!(s, Statement::Query(_))))),
      ),
      // newlines
      ("select *\nfrom users;", Ok(("SELECT * FROM users".to_owned(), Box::new(|s| matches!(s, Statement::Query(_)))))),
      // comment-only
      ("-- select * from users;", Err(ParseError::EmptyQuery("Parsed query is empty".to_owned()))),
      // commented line(s)
      (
        "-- select blah;\nselect * from users",
        Ok(("SELECT * FROM users".to_owned(), Box::new(|s| matches!(s, Statement::Query(_))))),
      ),
      // update
      (
        "UPDATE users SET name = 'John' WHERE id = 1",
        Ok((
          "UPDATE users SET name = 'John' WHERE id = 1".to_owned(),
          Box::new(|s| matches!(s, Statement::Update { .. })),
        )),
      ),
      // delete
      (
        "DELETE FROM users WHERE id = 1",
        Ok(("DELETE FROM users WHERE id = 1".to_owned(), Box::new(|s| matches!(s, Statement::Delete { .. })))),
      ),
      // drop
      ("DROP TABLE users", Ok(("DROP TABLE users".to_owned(), Box::new(|s| matches!(s, Statement::Drop { .. }))))),
      // explain
      (
        "EXPLAIN SELECT * FROM users",
        Ok(("EXPLAIN SELECT * FROM users".to_owned(), Box::new(|s| matches!(s, Statement::Explain { .. })))),
      ),
    ];

    let dialect = Box::new(SQLiteDialect {});

    for (input, expected_output) in test_cases {
      let result = get_first_query(input.to_string(), Driver::Sqlite);
      match (result, expected_output) {
        (Ok((query, statement_type)), Ok((expected_query, match_statement))) => {
          assert_eq!(query, expected_query);
          assert!(match_statement(statement_type));
        },
        (Err(ParseError::EmptyQuery(msg)), Err(ParseError::EmptyQuery(expected_msg))) => {
          assert_eq!(msg, expected_msg)
        },
        (Err(ParseError::MoreThanOneStatement(msg)), Err(ParseError::MoreThanOneStatement(expected_msg))) => {
          assert_eq!(msg, expected_msg)
        },
        (Err(ParseError::SqlParserError(msg)), Err(ParseError::SqlParserError(expected_msg))) => {
          assert_eq!(msg, expected_msg)
        },
        _ => panic!("Unexpected result for input: {input}"),
      }
    }
  }

  #[test]
  fn test_execution_type_sqlite() {
    let test_cases = vec![
      ("DELETE FROM users WHERE id = 1", ExecutionType::Transaction),
      ("DROP TABLE users", ExecutionType::Confirm),
      ("UPDATE users SET name = 'John' WHERE id = 1", ExecutionType::Transaction),
      ("SELECT * FROM users", ExecutionType::Normal),
      ("INSERT INTO users (name) VALUES ('John')", ExecutionType::Normal),
      ("EXPLAIN DELETE FROM users WHERE id = 1", ExecutionType::Normal),
      ("EXPLAIN SELECT * FROM users", ExecutionType::Normal),
      ("EXPLAIN QUERY PLAN SELECT * FROM users", ExecutionType::Normal),
      ("EXPLAIN Query PLAN DELETE FROM users WHERE id = 1", ExecutionType::Normal),
    ];

    for (query, expected) in test_cases {
      assert_eq!(
        get_execution_type(query.to_string(), false, Driver::Sqlite).unwrap().0,
        expected,
        "Failed for query: {query}"
      );
    }
  }
}
