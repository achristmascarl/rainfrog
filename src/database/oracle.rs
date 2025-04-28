use std::sync::Arc;

use async_trait::async_trait;
use color_eyre::eyre::Result;
use oracle::Connection;
use sqlparser::dialect::Dialect;

use crate::cli::Driver;

use super::{Database, DbTaskResult, Header, QueryResultsWithMetadata, QueryTask, Rows};

#[derive(Debug)]
pub struct OracleDialect {}
impl Dialect for OracleDialect {
  fn is_identifier_start(&self, c: char) -> bool {
    c.is_alphabetic() || c == '_'
  }
  fn is_identifier_part(&self, ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '$' || ch == '#'
  }
}

enum OracleTask {
  Query(QueryTask),
}

#[derive(Default)]
pub struct OracleDriver {
  conn: Option<Arc<Connection>>,
  task: Option<OracleTask>,
}

#[async_trait(?Send)]
impl Database for OracleDriver {
  async fn init(&mut self, args: crate::cli::Cli) -> Result<()> {
    let crate::cli::Cli { mouse_mode, connection_url, user, password, host, port, database, driver } = args;
    let host = host.unwrap_or_else(|| "localhost".to_string());
    let port = port.unwrap_or(1521);
    let user = user.unwrap_or_else(|| "rainfrog".to_string());
    let password = password.unwrap_or_else(|| "password".to_string());
    let database = database.unwrap_or_else(|| "rainfrog".to_string());

    let connection_string = format!("//{}:{}/{}", host, port, database);
    let connection = oracle::Connection::connect(user, password, connection_string).unwrap();
    self.conn = Some(Arc::new(connection));

    Ok(())
  }

  fn start_query(&mut self, query: String) -> Result<()> {
    let (first_query, statement_type) = super::get_first_query(query, Driver::Oracle)?;
    let conn = self.conn.clone();

    self.task = Some(OracleTask::Query(tokio::spawn(async move {
      let results = query_with_connection(&conn.unwrap(), &first_query);
      log::info!("results: {:?}", results);
      QueryResultsWithMetadata { results, statement_type }
    })));

    Ok(())
  }

  fn abort_query(&mut self) -> Result<bool> {
    if let Some(task) = self.task.take() {
      match task {
        OracleTask::Query(handle) => handle.abort(),
      };
      Ok(true)
    } else {
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
          (DbTaskResult::Finished(handle.await?), None)
        }
      },
    };
    self.task = next_task;
    Ok(task_result)
  }

  async fn start_tx(&mut self, query: String) -> Result<()> {
    Self::start_query(self, query)
  }

  async fn commit_tx(&mut self) -> Result<Option<QueryResultsWithMetadata>> {
    todo!();
  }

  async fn rollback_tx(&mut self) -> Result<()> {
    todo!();
  }

  async fn load_menu(&self) -> Result<Rows> {
    query_with_connection(self.conn.as_ref().unwrap(), "SELECT table_name, table_name FROM user_tables")
  }

  fn preview_rows_query(&self, schema: &str, table: &str) -> String {
    format!("SELECT * FROM \"{}\".\"{}\" ROWNUM <= 100", schema, table)
  }

  fn preview_columns_query(&self, schema: &str, table: &str) -> String {
    format!(
      "SELECT column_name, data_type, data_length FROM user_tab_columns WHERE table_name = '{}' AND owner = '{}'",
      table, schema
    )
  }

  fn preview_constraints_query(&self, schema: &str, table: &str) -> String {
    format!( "SELECT constraint_name, constraint_type, search_condition FROM user_constraints WHERE table_name = '{}' AND owner = '{}'", table, schema)
  }

  fn preview_indexes_query(&self, schema: &str, table: &str) -> String {
    format!(
      "SELECT index_name, uniqueness, column_name FROM user_ind_columns WHERE table_name = '{}' AND owner = '{}'",
      table, schema
    )
  }

  fn preview_policies_query(&self, schema: &str, table: &str) -> String {
    format!(
      "SELECT policy_name, object_name, policy_type FROM user_policies WHERE object_name = '{}' AND owner = '{}'",
      table, schema
    )
  }
}

fn query_with_connection(conn: &Connection, query: &str) -> Result<Rows> {
  let mut headers = vec![];
  let rows_affected = None;
  let rows = conn
    .query(&query, &[])
    .map_err(|e| color_eyre::eyre::eyre!("Error executing query: {}", e))?
    .filter_map(|row| row.ok())
    .map(|row| {
      if headers.is_empty() {
        headers = row
          .column_info()
          .iter()
          .map(|col| Header { name: col.name().to_string(), type_name: col.oracle_type().to_string() })
          .collect();
      }

      row.sql_values().iter().map(|v| v.to_string()).collect()
    })
    .collect::<Vec<_>>();
  Ok(Rows { headers, rows, rows_affected })
}

impl OracleDriver {
  pub fn new() -> Self {
    OracleDriver { conn: None, task: None }
  }
}
