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
    todo!()
  }

  fn preview_columns_query(&self, schema: &str, table: &str) -> String {
    todo!()
  }

  fn preview_constraints_query(&self, schema: &str, table: &str) -> String {
    todo!()
  }

  fn preview_indexes_query(&self, schema: &str, table: &str) -> String {
    todo!()
  }

  fn preview_policies_query(&self, schema: &str, table: &str) -> String {
    todo!()
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
