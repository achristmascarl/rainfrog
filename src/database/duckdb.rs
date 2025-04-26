use std::{
  io::{self, Write as _},
  string::String,
  sync::Arc,
};

use async_trait::async_trait;
use color_eyre::eyre::{self, Result};
use duckdb::{params, Config, Connection, Transaction};

use crate::cli::{Cli, Driver};

use super::{Database, DbTaskResult, Header, Headers, QueryResultsWithMetadata, QueryTask, Rows};

type TransactionTask<'a> = tokio::task::JoinHandle<(QueryResultsWithMetadata, Transaction<'a>)>;
enum DuckDbTask<'a> {
  Query(QueryTask),
  TxStart(TransactionTask<'a>),
  TxPending((Transaction<'a>, QueryResultsWithMetadata)),
}

#[derive(Default)]
pub struct DuckDbDriver<'a> {
  connection: Option<Arc<Connection>>,
  task: Option<DuckDbTask<'a>>,
}

#[async_trait(?Send)]
impl Database for DuckDbDriver<'_> {
  async fn init(&mut self, args: Cli) -> Result<()> {
    let (path, config) = super::DuckDbDriver::build_connection_opts(args)?;
    let conn = Arc::new(Connection::open_with_flags(path, config)?);
    self.connection = Some(conn);
    Ok(())
  }

  // since it's possible for raw_sql to execute multiple queries in a single string,
  // we only execute the first one and then drop the rest.
  fn start_query(&mut self, query: String) -> Result<()> {
    let (first_query, statement_type) = super::get_first_query(query, Driver::DuckDb)?;
    let connection = self.connection.clone().unwrap().try_clone()?;
    self.task = Some(DuckDbTask::Query(tokio::spawn(async move {
      let mut statement = connection.prepare(&first_query).unwrap();
      let mut headers: Headers = Vec::new();
      let mut results: Vec<Vec<String>> = Vec::new();
      let rows = statement.query([]);
      match rows {
        Ok(mut rows) => {
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
          QueryResultsWithMetadata { results: Ok(Rows { headers, rows: results, rows_affected: None }), statement_type }
        },
        Err(e) => {
          return QueryResultsWithMetadata { results: Err(eyre::Report::new(e)), statement_type };
        },
      }
    })));
    Ok(())
  }

  fn abort_query(&mut self) -> Result<bool> {
    if let Some(task) = self.task.take() {
      match task {
        DuckDbTask::Query(handle) => handle.abort(),
        DuckDbTask::TxStart(handle) => handle.abort(),
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
      Some(DuckDbTask::Query(handle)) => {
        if !handle.is_finished() {
          (DbTaskResult::Pending, Some(DuckDbTask::Query(handle)))
        } else {
          let result = handle.await?;
          (DbTaskResult::Finished(result), None)
        }
      },
      Some(DuckDbTask::TxStart(handle)) => {
        if !handle.is_finished() {
          (DbTaskResult::Pending, Some(DuckDbTask::TxStart(handle)))
        } else {
          let (result, tx) = handle.await?;
          let rows_affected = match &result.results {
            Ok(rows) => rows.rows_affected,
            _ => None,
          };
          (
            DbTaskResult::ConfirmTx(rows_affected, result.statement_type.clone()),
            Some(DuckDbTask::TxPending((tx, result))),
          )
        }
      },
      Some(DuckDbTask::TxPending((tx, results))) => (DbTaskResult::Pending, Some(DuckDbTask::TxPending((tx, results)))),
    };
    self.task = next_task;
    Ok(task_result)
  }

  async fn start_tx(&mut self, query: String) -> Result<()> {
    todo!()
  }

  async fn commit_tx(&mut self) -> Result<Option<QueryResultsWithMetadata>> {
    todo!()
  }

  async fn rollback_tx(&mut self) -> Result<()> {
    todo!()
  }

  async fn load_menu(&self) -> Result<Rows> {
    let connection = self.connection.clone().unwrap().try_clone()?;
    let mut statement = connection.prepare(
      "select table_schema, table_name
      from information_schema.tables
      where table_schema != 'information_schema'
      group by table_schema, table_name
      order by table_schema, table_name asc",
    )?;
    let mut rows = statement.query([])?;

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
}

impl DuckDbDriver<'_> {
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
