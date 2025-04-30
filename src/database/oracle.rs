use std::sync::Arc;

use async_trait::async_trait;
use color_eyre::eyre::Result;
use oracle::Connection;
use sqlparser::ast::Statement;

use crate::cli::Driver;

use super::{Database, DbTaskResult, Header, QueryResultsWithMetadata, QueryTask, Rows};

enum OracleTask {
  Query(QueryTask),
  TxStart(QueryTask), // FIXME: This should be a different type
}

#[derive(Default)]
pub struct OracleDriver {
  conn: Option<Arc<Connection>>,
  task: Option<OracleTask>,
}

impl OracleDriver {
  pub fn new() -> Self {
    OracleDriver { conn: None, task: None }
  }
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
    let conn = self.conn.clone().unwrap();

    let task = match statement_type {
      Statement::Query(_) => OracleTask::Query(tokio::spawn(async move {
        let results = query_with_connection(&conn, &first_query);
        QueryResultsWithMetadata { results, statement_type }
      })),
      _ => OracleTask::TxStart(tokio::spawn(async move {
        let results = execute_with_connection(&conn, &first_query);
        match results {
          Ok(ref rows) => {
            log::info!("{:?} rows, {:?} affected", rows.rows.len(), rows.rows_affected);
          },
          Err(ref e) => {
            log::error!("{e:?}");
          },
        };
        QueryResultsWithMetadata { results, statement_type }
      })),
    };

    self.task = Some(task);

    Ok(())
  }

  fn abort_query(&mut self) -> Result<bool> {
    if let Some(task) = self.task.take() {
      match task {
        OracleTask::Query(handle) => handle.abort(),
        OracleTask::TxStart(handle) => handle.abort(),
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
      Some(OracleTask::TxStart(handle)) => {
        if !handle.is_finished() {
          (DbTaskResult::Pending, Some(OracleTask::TxStart(handle)))
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
    log::info!("Committing transaction");
    todo!();
  }

  async fn rollback_tx(&mut self) -> Result<()> {
    todo!();
  }

  async fn load_menu(&self) -> Result<Rows> {
    query_with_connection(
      self.conn.as_ref().unwrap(),
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

fn query_with_connection(conn: &Connection, query: &str) -> Result<Rows> {
  let mut headers = Vec::new();
  let rows = conn
    .query(&query, &[])
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

fn execute_with_connection(conn: &Connection, statement: &str) -> Result<Rows> {
  let result = conn.execute(statement, &[]).map_err(|e| color_eyre::eyre::eyre!("Error executing statement: {}", e))?;
  conn.commit()?;

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
