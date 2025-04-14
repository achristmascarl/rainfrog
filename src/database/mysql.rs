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
  mysql::{MySql, MySqlConnectOptions, MySqlPoolOptions},
  Column, Either, Row, ValueRef,
};
use tokio::task::JoinHandle;

use super::{Database, DbTaskResult, Driver, Header, Headers, QueryResultsWithMetadata, QueryTask, Rows, Value};

type MySqlTransaction<'a> = sqlx::Transaction<'a, MySql>;
type TransactionTask<'a> = JoinHandle<(QueryResultsWithMetadata, MySqlTransaction<'a>)>;
enum MySqlTask<'a> {
  Query(QueryTask),
  TxStart(TransactionTask<'a>),
  TxPending((MySqlTransaction<'a>, QueryResultsWithMetadata)),
}

#[derive(Default)]
pub struct MySqlDriver<'a> {
  pool: Option<Arc<sqlx::Pool<MySql>>>,
  task: Option<MySqlTask<'a>>,
}

#[async_trait(?Send)]
impl Database for MySqlDriver<'_> {
  async fn init(&mut self, args: crate::cli::Cli) -> Result<()> {
    let opts = super::mysql::MySqlDriver::<'_>::build_connection_opts(args)?;
    let pool = Arc::new(MySqlPoolOptions::new().max_connections(3).connect_with(opts).await?);
    self.pool = Some(pool);
    Ok(())
  }

  // since it's possible for raw_sql to execute multiple queries in a single string,
  // we only execute the first one and then drop the rest.
  fn start_query(&mut self, query: String) -> Result<()> {
    let (first_query, statement_type) = super::get_first_query(query, Driver::MySql)?;
    let pool = self.pool.clone().unwrap();
    self.task = Some(MySqlTask::Query(tokio::spawn(async move {
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

  fn abort_query(&mut self) -> Result<bool> {
    if let Some(task) = self.task.take() {
      match task {
        MySqlTask::Query(handle) => handle.abort(),
        MySqlTask::TxStart(handle) => handle.abort(),
        _ => {},
      };
      Ok(true)
    } else {
      Ok(false)
    }
  }

  async fn get_query_results(&mut self) -> Result<DbTaskResult> {
    let (task_result, next_task) = match self.task.take() {
      None => (DbTaskResult::NoTask, None),
      Some(MySqlTask::Query(handle)) => {
        if !handle.is_finished() {
          (DbTaskResult::Pending, Some(MySqlTask::Query(handle)))
        } else {
          let result = handle.await?;
          (DbTaskResult::Finished(result), None)
        }
      },
      Some(MySqlTask::TxStart(handle)) => {
        if !handle.is_finished() {
          (DbTaskResult::Pending, Some(MySqlTask::TxStart(handle)))
        } else {
          let (result, tx) = handle.await?;
          let rows_affected = match &result.results {
            Ok(rows) => rows.rows_affected,
            _ => None,
          };
          (
            DbTaskResult::ConfirmTx(rows_affected, result.statement_type.clone()),
            Some(MySqlTask::TxPending((tx, result))),
          )
        }
      },
      Some(MySqlTask::TxPending((tx, results))) => (DbTaskResult::Pending, Some(MySqlTask::TxPending((tx, results)))),
    };
    self.task = next_task;
    Ok(task_result)
  }

  async fn start_tx(&mut self, query: String) -> Result<()> {
    let (first_query, statement_type) = super::get_first_query(query, Driver::MySql)?;
    let tx = self.pool.as_mut().unwrap().begin().await?;
    self.task = Some(MySqlTask::TxStart(tokio::spawn(async move {
      let (results, tx) = query_with_tx(tx, &first_query).await;
      match results {
        Ok(Either::Left(rows_affected)) => {
          log::info!("{:?} rows affected", rows_affected);
          (
            QueryResultsWithMetadata {
              results: Ok(Rows { headers: vec![], rows: vec![], rows_affected: Some(rows_affected) }),
              statement_type,
            },
            tx,
          )
        },
        Ok(Either::Right(rows)) => {
          log::info!("{:?} rows affected", rows.rows_affected);
          (QueryResultsWithMetadata { results: Ok(rows), statement_type }, tx)
        },
        Err(e) => {
          log::error!("{e:?}");
          (QueryResultsWithMetadata { results: Err(e), statement_type }, tx)
        },
      }
    })));
    Ok(())
  }

  async fn commit_tx(&mut self) -> Result<Option<QueryResultsWithMetadata>> {
    if !matches!(self.task, Some(MySqlTask::TxPending(_))) {
      Ok(None)
    } else if let Some(MySqlTask::TxPending((tx, results))) = self.task.take() {
      tx.commit().await?;
      Ok(Some(results))
    } else {
      Ok(None)
    }
  }

  async fn rollback_tx(&mut self) -> Result<()> {
    if let Some(MySqlTask::TxPending((tx, _))) = self.task.take() {
      tx.rollback().await?;
    }
    Ok(())
  }

  async fn load_menu(&self) -> Result<Rows> {
    query_with_pool(
      self.pool.clone().unwrap(),
      "select table_schema as table_schema, table_name as table_name
      from information_schema.tables
      where table_schema not in ('mysql', 'information_schema', 'performance_schema', 'sys')
      order by table_schema, table_name asc"
        .to_owned(),
    )
    .await
  }

  fn preview_rows_query(&self, schema: &str, table: &str) -> String {
    format!("select * from `{}`.`{}` limit 100", schema, table)
  }

  fn preview_columns_query(&self, schema: &str, table: &str) -> String {
    format!(
      "select column_name, data_type, is_nullable, column_default, extra, column_comment
        from information_schema.columns
        where table_schema = '{}' and table_name = '{}'
        order by ordinal_position",
      schema, table
    )
  }

  fn preview_constraints_query(&self, schema: &str, table: &str) -> String {
    format!(
      "select constraint_name, constraint_type, enforced,
        group_concat(column_name order by ordinal_position) as column_names
        from information_schema.table_constraints
        join information_schema.key_column_usage using (constraint_schema, constraint_name, table_schema, table_name)
        where table_schema = '{}' and table_name = '{}'
        group by constraint_name, constraint_type, enforced
        order by constraint_type, constraint_name",
      schema, table
    )
  }

  fn preview_indexes_query(&self, schema: &str, table: &str) -> String {
    format!(
      "select index_name, column_name, non_unique, seq_in_index, index_type
        from information_schema.statistics
        where table_schema = '{}' and table_name = '{}'
        order by index_name, seq_in_index",
      schema, table
    )
  }

  fn preview_policies_query(&self, schema: &str, table: &str) -> String {
    "select 'MySQL does not support row-level security policies' as message".to_owned()
  }
}

impl MySqlDriver<'_> {
  pub fn new() -> Self {
    Self { pool: None, task: None }
  }

  fn build_connection_opts(
    args: crate::cli::Cli,
  ) -> Result<<<sqlx::MySql as sqlx::Database>::Connection as sqlx::Connection>::Options> {
    match args.connection_url {
      Some(url) => Ok(MySqlConnectOptions::from_str(&url)?),
      None => {
        let mut opts = MySqlConnectOptions::new();

        // Username
        if let Some(user) = args.user {
          opts = opts.username(&user);
        } else {
          let mut user = String::new();
          print!("username: ");
          io::stdout().flush()?;
          io::stdin().read_line(&mut user)?;
          let user = user.trim();
          if !user.is_empty() {
            opts = opts.username(user);
          }
        }

        // Password
        if let Some(password) = args.password {
          opts = opts.password(&password);
        } else {
          let password = rpassword::prompt_password(format!("password for user {}: ", opts.get_username())).unwrap();
          let password = password.trim();
          if !password.is_empty() {
            opts = opts.password(password);
          }
        }

        // Host
        if let Some(host) = args.host {
          opts = opts.host(&host);
        } else {
          let mut host = String::new();
          print!("host (ex. localhost): ");
          io::stdout().flush()?;
          io::stdin().read_line(&mut host)?;
          let host = host.trim();
          if !host.is_empty() {
            opts = opts.host(host);
          }
        }

        // Port
        if let Some(port) = args.port {
          opts = opts.port(port);
        } else {
          let mut port = String::new();
          print!("port (ex. 3306): ");
          io::stdout().flush()?;
          io::stdin().read_line(&mut port)?;
          let port = port.trim();
          if !port.is_empty() {
            opts = opts.port(port.parse()?);
          }
        }

        // Database
        if let Some(database) = args.database {
          opts = opts.database(&database);
        } else {
          let mut database = String::new();
          print!("database (ex. mydb): ");
          io::stdout().flush()?;
          io::stdin().read_line(&mut database)?;
          let database = database.trim();
          if !database.is_empty() {
            opts = opts.database(database);
          }
        }

        Ok(opts)
      },
    }
  }
}

async fn query_with_pool(pool: Arc<sqlx::Pool<MySql>>, query: String) -> Result<Rows> {
  query_with_stream(&*pool.clone(), &query).await
}

async fn query_with_stream<'a, E>(e: E, query: &'a str) -> Result<Rows>
where
  E: sqlx::Executor<'a, Database = sqlx::MySql>,
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
  mut tx: MySqlTransaction<'static>,
  query: &str,
) -> (Result<Either<u64, Rows>>, MySqlTransaction<'static>)
where
  for<'c> <sqlx::MySql as sqlx::Database>::Arguments<'c>: sqlx::IntoArguments<'c, sqlx::MySql>,
  for<'c> &'c mut <sqlx::MySql as sqlx::Database>::Connection: sqlx::Executor<'c, Database = sqlx::MySql>,
{
  let first_query = super::get_first_query(query.to_string(), Driver::MySql);
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

fn get_headers(row: &<sqlx::MySql as sqlx::Database>::Row) -> Headers {
  row
    .columns()
    .iter()
    .map(|col| Header { name: col.name().to_string(), type_name: col.type_info().to_string() })
    .collect()
}

fn row_to_vec(row: &<sqlx::MySql as sqlx::Database>::Row) -> Vec<String> {
  row.columns().iter().map(|col| parse_value(row, col).unwrap().string).collect()
}

// parsed based on https://docs.rs/sqlx/latest/sqlx/mysql/types/index.html
fn parse_value(row: &<MySql as sqlx::Database>::Row, col: &<MySql as sqlx::Database>::Column) -> Option<Value> {
  let col_type = col.type_info().to_string();
  if row.try_get_raw(col.ordinal()).is_ok_and(|v| v.is_null()) {
    return Some(Value { parse_error: false, string: "NULL".to_string(), is_null: true });
  }
  match col_type.to_uppercase().as_str() {
    "TINYINT(1)" | "BOOLEAN" | "BOOL" => Some(row.try_get::<bool, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "TINYINT" => Some(row.try_get::<i8, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "SMALLINT" => Some(row.try_get::<i16, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "INT" => Some(row.try_get::<i32, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "BIGINT" => Some(row.try_get::<i64, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "TINYINT UNSIGNED" => Some(row.try_get::<u8, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "SMALLINT UNSIGNED" => Some(row.try_get::<u16, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "INT UNSIGNED" => Some(row.try_get::<u32, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "BIGINT UNSIGNED" => Some(row.try_get::<u64, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "FLOAT" => Some(row.try_get::<f32, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "DOUBLE" => Some(row.try_get::<f64, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "VARCHAR" | "CHAR" | "TEXT" | "BINARY" => Some(row.try_get::<String, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "VARBINARY" | "BLOB" => Some(row.try_get::<Vec<u8>, usize>(col.ordinal()).map_or(
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
    "INET4" | "INET6" => Some(row.try_get::<std::net::IpAddr, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "TIME" => Some(row.try_get::<chrono::NaiveTime, usize>(col.ordinal()).map_or(
      row.try_get::<chrono::TimeDelta, usize>(col.ordinal()).map_or(
        Value { parse_error: true, string: "_ERROR_".to_string(), is_null: true },
        |received| Value { parse_error: false, string: received.to_string(), is_null: false },
      ),
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "DATE" => Some(row.try_get::<chrono::NaiveDate, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "DATETIME" => Some(row.try_get::<chrono::NaiveDateTime, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "TIMESTAMP" => Some(row.try_get::<chrono::DateTime<chrono::Utc>, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "JSON" => Some(row.try_get::<serde_json::Value, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "GEOMETRY" => {
      // TODO: would have to resort to geozero to parse WKB
      Some(Value { parse_error: true, string: "_TODO_".to_owned(), is_null: false })
    },
    _ => {
      // Try to cast custom or other types to strings
      Some(row.try_get_unchecked::<String, usize>(col.ordinal()).map_or(
        Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
        |received| Value { parse_error: false, string: received.to_string(), is_null: false },
      ))
    },
  }
}

#[cfg(test)]
mod tests {
  use sqlparser::{ast::Statement, dialect::MySqlDialect, parser::ParserError};

  use super::*;
  use crate::database::{get_execution_type, get_first_query, ExecutionType, ParseError};

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

    let dialect = Box::new(MySqlDialect {});

    for (input, expected_output) in test_cases {
      let result = get_first_query(input.to_string(), Driver::MySql);
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
        _ => panic!("Unexpected result for input: {}", input),
      }
    }
  }

  #[test]
  fn test_execution_type_mysql() {
    let test_cases = vec![
      ("DELETE FROM users WHERE id = 1", ExecutionType::Transaction),
      ("DROP TABLE users", ExecutionType::Confirm),
      ("UPDATE users SET name = 'John' WHERE id = 1", ExecutionType::Transaction),
      ("SELECT * FROM users", ExecutionType::Normal),
      ("INSERT INTO users (name) VALUES ('John')", ExecutionType::Normal),
      ("EXPLAIN ANALYZE DELETE FROM users WHERE id = 1", ExecutionType::Transaction),
      ("EXPLAIN ANALYZE DROP TABLE users", ExecutionType::Confirm),
      ("EXPLAIN SELECT * FROM users", ExecutionType::Normal),
      ("EXPLAIN ANALYZE SELECT * FROM users WHERE id = 1", ExecutionType::Normal),
    ];

    for (query, expected) in test_cases {
      assert_eq!(
        get_execution_type(query.to_string(), false, Driver::MySql).unwrap().0,
        expected,
        "Failed for query: {}",
        query
      );
    }
  }
}
