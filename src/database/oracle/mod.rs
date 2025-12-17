mod connect_options;

use std::sync::Arc;

use async_trait::async_trait;
use color_eyre::eyre::Result;
use connect_options::OracleConnectOptions;
use oracle::{Connection, pool::Pool};
use sqlparser::ast::Statement;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::cli::Driver;

use super::{Database, DbTaskResult, Header, QueryResultsWithMetadata, QueryTask, Rows};

type TransactionTask = JoinHandle<Result<QueryResultsWithMetadata>>;
enum OracleTask {
  Query(QueryTask),
  TxStart(TransactionTask),
  TxPending(Box<QueryResultsWithMetadata>),
}

#[derive(Default)]
pub struct OracleDriver {
  pool: Option<Arc<oracle::pool::Pool>>,
  task: Option<OracleTask>,
  querying_conn: Option<Arc<Mutex<Connection>>>,
}

impl OracleDriver {
  pub fn new() -> Self {
    OracleDriver { pool: None, task: None, querying_conn: None }
  }
}

#[async_trait(?Send)]
impl Database for OracleDriver {
  async fn init(&mut self, args: crate::cli::Cli) -> Result<()> {
    let connection_opts = OracleConnectOptions::build_connection_opts(args)?;

    let (user, password, connection_string) =
      connection_opts.get_connection_options().map_err(|e| color_eyre::eyre::eyre!(e))?;
    let pool = Arc::new(oracle::pool::PoolBuilder::new(user, password, connection_string).max_connections(3).build()?);
    self.pool = Some(pool);

    Ok(())
  }

  async fn start_query(&mut self, query: String, bypass_parser: bool) -> Result<()> {
    let (first_query, statement_type) = if bypass_parser {
      (query, None)
    } else {
      let (first, stmt) = super::get_first_query(query, Driver::Oracle)?;
      (first, Some(stmt))
    };
    let pool = self.pool.clone().unwrap();

    let conn = Arc::new(Mutex::new(pool.get()?));
    let query_conn = conn.clone();
    self.querying_conn = Some(conn);
    let task = match statement_type {
      Some(Statement::Query(_)) => OracleTask::Query(tokio::spawn(async move {
        let c = query_conn.lock().await;
        let results = query_with_conn(&c, &first_query);
        QueryResultsWithMetadata { results, statement_type }
      })),
      _ => OracleTask::TxStart(tokio::spawn(async move {
        let c = query_conn.lock().await;
        let results = execute_with_conn(&c, &first_query);
        match results {
          Ok(ref rows) => {
            log::info!("{:?} rows, {:?} affected", rows.rows.len(), rows.rows_affected);
          },
          Err(ref e) => {
            log::error!("{e:?}");
          },
        };
        Ok(QueryResultsWithMetadata { results, statement_type })
      })),
    };

    self.task = Some(task);

    Ok(())
  }

  async fn abort_query(&mut self) -> Result<bool> {
    if let Some(task) = self.task.take() {
      match task {
        OracleTask::Query(handle) => handle.abort(),
        OracleTask::TxStart(handle) => handle.abort(),
        _ => {},
      };
      if let Some(conn) = &self.querying_conn {
        let c = conn.lock().await;
        let _ = c.break_execution();
      }
      self.querying_conn = None;
      Ok(true)
    } else {
      self.querying_conn = None;
      Ok(false)
    }
  }

  async fn get_query_results(&mut self) -> Result<DbTaskResult> {
    let (task_result, next_task) = match self.task.take() {
      None => (DbTaskResult::NoTask, None),
      Some(OracleTask::Query(handle)) => {
        if !handle.is_finished() {
          (DbTaskResult::Pending, Some(OracleTask::Query(handle)))
        } else {
          self.querying_conn = None;
          (DbTaskResult::Finished(handle.await?), None)
        }
      },
      Some(OracleTask::TxStart(handle)) => {
        if !handle.is_finished() {
          (DbTaskResult::Pending, Some(OracleTask::TxStart(handle)))
        } else {
          let result = handle.await??;
          let rows_affected = match &result.results {
            Ok(rows) => rows.rows_affected,
            _ => None,
          };
          match result {
            // if tx failed to start, return the error immediately
            QueryResultsWithMetadata { results: Err(e), statement_type } => {
              log::error!("Transaction didn't start: {e:?}");
              self.querying_conn = None;
              (DbTaskResult::Finished(QueryResultsWithMetadata { results: Err(e), statement_type }), None)
            },
            _ => (
              DbTaskResult::ConfirmTx(rows_affected, result.statement_type.clone()),
              Some(OracleTask::TxPending(Box::new(result))),
            ),
          }
        }
      },
      Some(OracleTask::TxPending(handle)) => (DbTaskResult::Pending, Some(OracleTask::TxPending(handle))),
    };
    self.task = next_task;
    Ok(task_result)
  }

  async fn start_tx(&mut self, query: String) -> Result<()> {
    Self::start_query(self, query, false).await
  }

  async fn commit_tx(&mut self) -> Result<Option<QueryResultsWithMetadata>> {
    if let Some(OracleTask::TxPending(b)) = self.task.take()
      && let Some(self_conn) = self.querying_conn.clone()
    {
      let conn = self_conn.lock().await;
      let result = conn.commit()?;
      self.querying_conn = None;
      Ok(Some(*b))
    } else {
      Ok(None)
    }
  }

  async fn rollback_tx(&mut self) -> Result<()> {
    if let Some(OracleTask::TxPending(b)) = self.task.take()
      && let Some(self_conn) = self.querying_conn.clone()
    {
      let conn = self_conn.lock().await;
      let result = conn.rollback()?;
      self.querying_conn = None;
      Ok(())
    } else {
      Ok(())
    }
  }

  async fn load_menu(&self) -> Result<Rows> {
    query_with_pool(
      self.pool.as_ref().unwrap(),
      "select user, table_name from user_tables where tablespace_name is not null order by user, table_name",
    )
  }

  fn preview_rows_query(&self, schema: &str, table: &str) -> String {
    format!("select * from \"{}\".\"{}\" where rownum <= 100", schema, table)
  }

  fn preview_columns_query(&self, schema: &str, table: &str) -> String {
    format!("select * from user_tab_columns where table_name = '{}' and user = '{}'", table, schema)
  }

  fn preview_constraints_query(&self, schema: &str, table: &str) -> String {
    format!("select * from user_constraints where table_name = '{}' and user = '{}'", table, schema)
  }

  fn preview_indexes_query(&self, schema: &str, table: &str) -> String {
    format!("select * from user_ind_columns where table_name = '{}' and user = '{}'", table, schema)
  }

  fn preview_policies_query(&self, schema: &str, table: &str) -> String {
    format!("select * from user_policies where object_name = '{}' and user = '{}'", table, schema)
  }
}

fn query_with_pool(pool: &Pool, query: &str) -> Result<Rows> {
  let conn = pool.get()?;
  query_with_conn(&conn, query)
}

fn query_with_conn(conn: &Connection, query: &str) -> Result<Rows> {
  let mut headers = Vec::new();
  let rows = conn
    .query(query, &[])
    .map_err(|e| color_eyre::eyre::eyre!("Error executing query: {}", e))?
    .filter_map(|row| row.ok())
    .map(|row| {
      if headers.is_empty() {
        headers = get_headers(&row);
      }

      row_to_vec(&row)
    })
    .collect::<Vec<_>>();

  Ok(Rows { headers, rows, rows_affected: None })
}

fn execute_with_conn(conn: &Connection, statement: &str) -> Result<Rows> {
  let result = conn.execute(statement, &[]).map_err(|e| color_eyre::eyre::eyre!("Error executing statement: {}", e))?;
  Ok(Rows { headers: Vec::new(), rows: Vec::new(), rows_affected: result.row_count().ok() })
}

fn get_headers(row: &oracle::Row) -> Vec<Header> {
  row
    .column_info()
    .iter()
    .map(|col| Header { name: col.name().to_string(), type_name: col.oracle_type().to_string() })
    .collect()
}

fn row_to_vec(row: &oracle::Row) -> Vec<String> {
  row.sql_values().iter().map(|v| v.to_string()).collect()
}

#[cfg(test)]
mod tests {
  use sqlparser::{ast::Statement, parser::ParserError};

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

    for (input, expected_output) in test_cases {
      let result = get_first_query(input.to_string(), Driver::Oracle);
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
        get_execution_type(query.to_string(), false, Driver::Oracle).unwrap().0,
        expected,
        "Failed for query: {}",
        query
      );
    }
  }
}
