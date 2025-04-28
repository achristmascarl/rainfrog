use std::{
  io::{self, Write as _},
  string::String,
};

use async_trait::async_trait;
use color_eyre::eyre::{self, Result};
use duckdb::{Config, Connection};
use sqlparser::ast::Statement;

use crate::cli::{Cli, Driver};

use super::{Database, DbTaskResult, ExecutionType, Header, Headers, QueryResultsWithMetadata, QueryTask, Rows};

enum DuckDbTask {
  Query(QueryTask),
}

#[derive(Default)]
pub struct DuckDbDriver {
  connection: Option<Connection>,
  task: Option<DuckDbTask>,
}

#[async_trait(?Send)]
impl Database for DuckDbDriver {
  async fn init(&mut self, args: Cli) -> Result<()> {
    let (path, config) = super::DuckDbDriver::build_connection_opts(args)?;
    let conn = Connection::open_with_flags(path, config)?;
    self.connection = Some(conn);
    Ok(())
  }

  // since it's possible for raw_sql to execute multiple queries in a single string,
  // we only execute the first one and then drop the rest.
  fn start_query(&mut self, query: String) -> Result<()> {
    let (first_query, statement_type) = super::get_first_query(query.clone(), Driver::DuckDb)?;
    // since Connection isn't Send/Sync, we need to clone it for each query:
    // https://github.com/duckdb/duckdb-rs/issues/378
    let connection = self.connection.as_ref().unwrap().try_clone()?;
    self.task = Some(DuckDbTask::Query(tokio::spawn(async move {
      let results = run_query(connection, first_query).await;
      match results {
        Ok(rows) => QueryResultsWithMetadata { results: Ok(rows), statement_type },
        Err(e) => QueryResultsWithMetadata { results: Err(e), statement_type },
      }
    })));
    Ok(())
  }

  fn abort_query(&mut self) -> Result<bool> {
    if let Some(task) = self.task.take() {
      match task {
        DuckDbTask::Query(handle) => handle.abort(),
      };
      Ok(true)
    } else {
      Ok(false)
    }
  }

  async fn get_query_results(&mut self) -> Result<DbTaskResult> {
    let (task_result, next_task) = match self.task.take() {
      None => (DbTaskResult::NoTask, None),
      Some(DuckDbTask::Query(handle)) => {
        if !handle.is_finished() {
          (DbTaskResult::Pending, Some(DuckDbTask::Query(handle)))
        } else {
          let result = handle.await?;
          (DbTaskResult::Finished(result), None)
        }
      },
    };
    self.task = next_task;
    Ok(task_result)
  }

  async fn start_tx(&mut self, query: String) -> Result<()> {
    Err(eyre::Report::msg("Transactions are not currently supported when using the DuckDB driver"))
  }

  async fn commit_tx(&mut self) -> Result<Option<QueryResultsWithMetadata>> {
    Err(eyre::Report::msg("Transactions are not currently supported when using the DuckDB driver"))
  }

  async fn rollback_tx(&mut self) -> Result<()> {
    Err(eyre::Report::msg("Transactions are not currently supported when using the DuckDB driver"))
  }

  async fn load_menu(&self) -> Result<Rows> {
    let connection = self.connection.as_ref().unwrap().try_clone()?;
    run_query(
      connection,
      "select table_schema, table_name
      from information_schema.tables
      where table_schema != 'information_schema'
      group by table_schema, table_name
      order by table_schema, table_name asc"
        .to_string(),
    )
    .await
  }

  fn preview_rows_query(&self, schema: &str, table: &str) -> String {
    format!("select * from \"{}\".\"{}\" limit 100", schema, table)
  }

  fn preview_columns_query(&self, schema: &str, table: &str) -> String {
    format!(
      "select column_name, * from information_schema.columns where table_schema = '{}' and table_name = '{}'",
      schema, table
    )
  }

  fn preview_constraints_query(&self, schema: &str, table: &str) -> String {
    format!(
      "select constraint_name, * from information_schema.table_constraints where table_schema = '{}' and table_name = '{}'",
      schema, table
    )
  }

  fn preview_indexes_query(&self, schema: &str, table: &str) -> String {
    format!("select indexname, indexdef, * from pg_indexes where schemaname = '{}' and tablename = '{}'", schema, table)
  }

  fn preview_policies_query(&self, schema: &str, table: &str) -> String {
    "select 'DuckDB does not support row-level security policies' as message".to_owned()
  }

  fn get_execution_type(&self, query: String, confirmed: bool) -> Result<(ExecutionType, Statement)> {
    let (_, statement) = super::get_first_query(query, Driver::DuckDb)?;
    match super::get_default_execution_type(statement.clone(), confirmed) {
      ExecutionType::Normal => Ok((ExecutionType::Normal, statement)),
      ExecutionType::Confirm => Ok((ExecutionType::Confirm, statement)),
      ExecutionType::Transaction => Ok((ExecutionType::Confirm, statement)), // don't allow auto-transactions
    }
  }
}

async fn run_query(connection: Connection, query: String) -> Result<Rows> {
  let mut statement = connection.prepare(query.as_str())?;
  let rows = statement.query([])?;
  fetch_rows(rows)
}

fn fetch_rows(mut rows: duckdb::Rows<'_>) -> Result<Rows> {
  let mut headers: Headers = Vec::new();
  let mut results: Vec<Vec<String>> = Vec::new();
  while let Ok(Some(row)) = rows.next() {
    if headers.is_empty() {
      headers = row
        .as_ref()
        .column_names()
        .iter()
        .enumerate()
        .map(|(i, col)| {
          let type_name = row.as_ref().column_type(i);
          Header { type_name: type_name.to_string(), name: col.to_string() }
        })
        .collect();
    }
    let mut r: Vec<String> = Vec::new();
    for i in 0..headers.len() {
      let value = row.get::<_, Option<String>>(i);
      if let Ok(Some(value)) = value {
        r.push(value);
      } else {
        r.push(String::new());
      }
    }
    results.push(r);
  }
  Ok(Rows { headers, rows: results, rows_affected: None })
}

impl DuckDbDriver {
  pub fn new() -> Self {
    DuckDbDriver { connection: None, task: None }
  }

  fn build_connection_opts(args: crate::cli::Cli) -> Result<(String, Config)> {
    match args.connection_url {
      Some(url) => Ok((url, Config::default())),
      None => {
        if let Some(database) = args.database {
          Ok((database, Config::default()))
        } else {
          let mut database = String::new();
          print!("database file path (or ':memory:'): ");
          io::stdout().flush()?;
          io::stdin().read_line(&mut database)?;
          let database = database.trim().to_string();
          if database.is_empty() {
            Err(eyre::Report::msg("Database file path is required"))
          } else {
            Ok((database, Config::default()))
          }
        }
      },
    }
  }
}

#[cfg(test)]
mod tests {
  use sqlparser::{ast::Statement, parser::ParserError};

  use super::*;
  use crate::database::{get_first_query, ExecutionType, ParseError};

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
        _ => panic!("Unexpected result for input: {}", input),
      }
    }
  }

  #[test]
  fn test_execution_type_sqlite() {
    let test_cases = vec![
      ("DELETE FROM users WHERE id = 1", ExecutionType::Confirm),
      ("DROP TABLE users", ExecutionType::Confirm),
      ("UPDATE users SET name = 'John' WHERE id = 1", ExecutionType::Confirm),
      ("SELECT * FROM users", ExecutionType::Normal),
      ("INSERT INTO users (name) VALUES ('John')", ExecutionType::Normal),
      ("EXPLAIN DELETE FROM users WHERE id = 1", ExecutionType::Normal),
      ("EXPLAIN SELECT * FROM users", ExecutionType::Normal),
      ("EXPLAIN QUERY PLAN SELECT * FROM users", ExecutionType::Normal),
      ("EXPLAIN Query PLAN DELETE FROM users WHERE id = 1", ExecutionType::Normal),
      (
        "SELECT * FROM read_csv('flights.csv', delim = '|', header = true, columns = {
          'FlightDate': 'DATE',
          'UniqueCarrier': 'VARCHAR',
          'OriginCityName': 'VARCHAR',
          'DestCityName': 'VARCHAR'
        });",
        ExecutionType::Normal,
      ),
      // TODO: Uncomment once https://github.com/apache/datafusion-sqlparser-rs/issues/1824 is fixed
      // ("COPY FROM DATABASE memory TO my_database;", ExecutionType::Normal),
    ];

    let driver = DuckDbDriver::new();

    for (query, expected) in test_cases {
      assert_eq!(
        driver.get_execution_type(query.to_string(), false).unwrap().0,
        expected,
        "Failed for query: {}",
        query
      );
    }
  }
}
