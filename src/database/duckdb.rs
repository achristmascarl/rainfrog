use std::{
  io::{self, Write as _},
  string::String,
  sync::Arc,
};

use async_trait::async_trait;
use color_eyre::eyre::{self, Result};
use duckdb::{params, Config, Connection, Transaction};

use crate::cli::{Cli, Driver};

use super::{Database, Header, Headers, QueryResultsWithMetadata, QueryTask};

type TransactionTask<'a> = tokio::task::JoinHandle<(QueryResultsWithMetadata, Transaction<'a>)>;
enum DuckDbTask<'a> {
  Query(QueryTask),
  TxStart(TransactionTask<'a>),
  TxPending((Transaction<'a>, QueryResultsWithMetadata)),
}

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
    let connection = self.connection.clone().unwrap();
    let statement = connection.prepare(&first_query)?;
    self.task = Some(DuckDbTask::Query(tokio::spawn(async move {
      let headers: Headers = statement
        .column_names()
        .iter()
        .enumerate()
        .map(|(i, col)| {
          let type_name = statement.column_type(i);
          Header { type_name: type_name.to_string(), name: col.to_string() }
        })
        .collect();

      let mut rows = statement.query([])?;
      // let results = query_with_pool(pool, first_query.clone()).await;
      // match results {
      //   Ok(ref rows) => {
      //     log::info!("{:?} rows, {:?} affected", rows.rows.len(), rows.rows_affected);
      //   },
      //   Err(ref e) => {
      //     log::error!("{e:?}");
      //   },
      // };
      // QueryResultsWithMetadata { results, statement_type: statement_type.clone() }
    })));
    Ok(())
  }

  fn abort_query(&mut self) -> Result<bool>;

  async fn get_query_results(&mut self) -> Result<DbTaskResult>;

  async fn start_tx(&mut self, query: String) -> Result<()>;

  async fn commit_tx(&mut self) -> Result<Option<QueryResultsWithMetadata>>;

  async fn rollback_tx(&mut self) -> Result<()>;

  async fn load_menu(&self) -> Result<Rows>;

  fn preview_rows_query(&self, schema: &str, table: &str) -> String;

  fn preview_columns_query(&self, schema: &str, table: &str) -> String;

  fn preview_constraints_query(&self, schema: &str, table: &str) -> String;

  fn preview_indexes_query(&self, schema: &str, table: &str) -> String;

  fn preview_policies_query(&self, schema: &str, table: &str) -> String;
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
