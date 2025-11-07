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
  pool::PoolConnection,
  postgres::{PgConnectOptions, PgConnection, PgPoolOptions, Postgres},
  types::Uuid,
};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use super::{
  Database, DbTaskResult, Driver, Header, Headers, QueryResultsWithMetadata, QueryTask, Rows, Value, vec_to_string,
};

type PostgresTransaction<'a> = sqlx::Transaction<'a, Postgres>;
type TransactionTask<'a> = JoinHandle<(QueryResultsWithMetadata, PostgresTransaction<'a>)>;
enum PostgresTask<'a> {
  Query(QueryTask),
  TxStart(TransactionTask<'a>),
  TxPending(Box<(PostgresTransaction<'a>, QueryResultsWithMetadata)>),
}

#[derive(Default)]
pub struct PostgresDriver<'a> {
  pool: Option<Arc<sqlx::Pool<Postgres>>>,
  task: Option<PostgresTask<'a>>,
  querying_conn: Option<Arc<Mutex<PoolConnection<Postgres>>>>,
  querying_pid: Option<String>,
}

#[async_trait(?Send)]
impl Database for PostgresDriver<'_> {
  async fn init(&mut self, args: crate::cli::Cli) -> Result<()> {
    let opts = super::postgresql::PostgresDriver::<'_>::build_connection_opts(args)?;
    let pool = Arc::new(PgPoolOptions::new().max_connections(3).connect_with(opts).await?);
    self.pool = Some(pool);
    Ok(())
  }

  // since it's possible for raw_sql to execute multiple queries in a single string,
  // we only execute the first one and then drop the rest.
  async fn start_query(&mut self, query: String, bypass_parser: bool) -> Result<()> {
    let (first_query, statement_type) = match bypass_parser {
      true => (query, None),
      false => {
        let (first, stmt) = super::get_first_query(query, Driver::Postgres)?;
        (first, Some(stmt))
      },
    };
    let pool = self.pool.clone().unwrap();
    self.querying_conn = Some(Arc::new(Mutex::new(pool.acquire().await?)));
    let conn = self.querying_conn.clone().unwrap();
    let conn_for_task = conn.clone();
    let pid = sqlx::raw_sql("SELECT pg_backend_pid()").fetch_one(conn.lock().await.as_mut()).await?.get::<i32, _>(0);
    log::info!("Starting query with PID {}", pid.clone());
    self.querying_pid = Some(pid.to_string().clone());
    self.task = Some(PostgresTask::Query(tokio::spawn(async move {
      let results = query_with_conn(conn_for_task.lock().await.as_mut(), first_query.clone()).await;
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
          PostgresTask::Query(handle) => handle.abort(),
          PostgresTask::TxStart(handle) => handle.abort(),
          _ => {},
        };
        if let Some(pid) = self.querying_pid.take() {
          let result =
            sqlx::raw_sql(&format!("SELECT pg_cancel_backend({pid})")).fetch_one(&*self.pool.clone().unwrap()).await;
          let msg = match &result {
            Ok(_) => "Successfully killed".to_string(),
            Err(e) => format!("Failed to kill: {e:?}"),
          };

          let success = result.as_ref().is_ok_and(|r| r.try_get::<bool, _>(0).unwrap_or(false));
          let status_string =
            result.map_or("ERROR".to_string(), |r| r.try_get_unchecked::<String, _>(0).unwrap_or("ERROR".to_string()));

          if !success {
            log::warn!("Unexpected response when cancelling backend process with PID {pid}: {msg}");
            log::warn!("Status: {status_string}");
          } else {
            log::info!("Tried to cancel backend process with PID {pid}: {msg}");
            log::info!("Status: {status_string}");
          }
        }
        self.querying_conn = None;
        Ok(true)
      },
      _ => {
        self.querying_conn = None;
        self.querying_pid = None;
        Ok(false)
      },
    }
  }

  async fn get_query_results(&mut self) -> Result<DbTaskResult> {
    let (task_result, next_task) = match self.task.take() {
      None => (DbTaskResult::NoTask, None),
      Some(PostgresTask::Query(handle)) => {
        if !handle.is_finished() {
          (DbTaskResult::Pending, Some(PostgresTask::Query(handle)))
        } else {
          let result = handle.await?;
          self.querying_conn = None;
          self.querying_pid = None;
          (DbTaskResult::Finished(result), None)
        }
      },
      Some(PostgresTask::TxStart(handle)) => {
        if !handle.is_finished() {
          (DbTaskResult::Pending, Some(PostgresTask::TxStart(handle)))
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
              self.querying_conn = None;
              self.querying_pid = None;
              (DbTaskResult::Finished(QueryResultsWithMetadata { results: Err(e), statement_type }), None)
            },
            _ => (
              DbTaskResult::ConfirmTx(rows_affected, result.statement_type.clone()),
              Some(PostgresTask::TxPending(Box::new((tx, result)))),
            ),
          }
        }
      },
      Some(PostgresTask::TxPending(b)) => (DbTaskResult::Pending, Some(PostgresTask::TxPending(b))),
    };
    self.task = next_task;
    Ok(task_result)
  }

  async fn start_tx(&mut self, query: String) -> Result<()> {
    let (first_query, statement_type) = super::get_first_query(query, Driver::Postgres)?;
    let mut tx = self.pool.clone().unwrap().begin().await?;
    let pid = sqlx::raw_sql("SELECT pg_backend_pid()").fetch_one(&mut *tx).await?.get::<i32, _>(0);
    log::info!("Starting transaction with PID {}", pid.clone());
    self.querying_pid = Some(pid.to_string().clone());
    self.task = Some(PostgresTask::TxStart(tokio::spawn(async move {
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
    if !matches!(self.task, Some(PostgresTask::TxPending(_))) {
      Ok(None)
    } else {
      match self.task.take() {
        Some(PostgresTask::TxPending(b)) => {
          b.0.commit().await?;
          self.querying_conn = None;
          self.querying_pid = None;
          Ok(Some(b.1))
        },
        _ => Ok(None),
      }
    }
  }

  async fn rollback_tx(&mut self) -> Result<()> {
    if let Some(PostgresTask::TxPending(b)) = self.task.take() {
      b.0.rollback().await?;
    }
    self.querying_conn = None;
    self.querying_pid = None;
    Ok(())
  }

  async fn load_menu(&self) -> Result<Rows> {
    query_with_pool(
      self.pool.clone().unwrap(),
      "select table_schema, table_name
      from information_schema.tables
      where table_schema != 'pg_catalog'
      and table_schema != 'information_schema'
      group by table_schema, table_name
      order by table_schema, table_name asc"
        .to_owned(),
    )
    .await
  }

  fn preview_rows_query(&self, schema: &str, table: &str) -> String {
    format!("select * from \"{schema}\".\"{table}\" limit 100")
  }

  fn preview_columns_query(&self, schema: &str, table: &str) -> String {
    format!(
      "select column_name, * from information_schema.columns where table_schema = '{schema}' and table_name = '{table}'"
    )
  }

  fn preview_constraints_query(&self, schema: &str, table: &str) -> String {
    format!(
      "select constraint_name, * from information_schema.table_constraints where table_schema = '{schema}' and table_name = '{table}'"
    )
  }

  fn preview_indexes_query(&self, schema: &str, table: &str) -> String {
    format!("select indexname, indexdef, * from pg_indexes where schemaname = '{schema}' and tablename = '{table}'")
  }

  fn preview_policies_query(&self, schema: &str, table: &str) -> String {
    format!("select * from pg_policies where schemaname = '{schema}' and tablename = '{table}'")
  }
}

impl PostgresDriver<'_> {
  pub fn new() -> Self {
    Self { pool: None, task: None, querying_conn: None, querying_pid: None }
  }

  fn build_connection_opts(
    args: crate::cli::Cli,
  ) -> Result<<<sqlx::Postgres as sqlx::Database>::Connection as sqlx::Connection>::Options> {
    match args.connection_url {
      Some(url) => Ok(PgConnectOptions::from_str(url.trim().trim_start_matches("jdbc:"))?),
      None => {
        let mut opts = PgConnectOptions::new();

        if let Some(user) = args.user {
          opts = opts.username(&user);
        } else {
          let mut user: String = String::new();
          print!("username: ");
          io::stdout().flush().unwrap();
          io::stdin().read_line(&mut user).unwrap();
          user = user.trim().to_string();
          if !user.is_empty() {
            opts = opts.username(&user);
          }
        }

        if let Some(password) = args.password {
          opts = opts.password(&password);
        } else {
          let mut password =
            rpassword::prompt_password(format!("password for user {}: ", opts.get_username())).unwrap();
          password = password.trim().to_string();
          if !password.is_empty() {
            opts = opts.password(&password);
          }
        }

        if let Some(host) = args.host {
          opts = opts.host(&host);
        } else {
          let mut host: String = String::new();
          print!("host (ex. localhost): ");
          io::stdout().flush().unwrap();
          io::stdin().read_line(&mut host).unwrap();
          host = host.trim().to_string();
          if !host.is_empty() {
            opts = opts.host(&host);
          }
        }

        if let Some(port) = args.port {
          opts = opts.port(port);
        } else {
          let mut port: String = String::new();
          print!("port (ex. 5432): ");
          io::stdout().flush().unwrap();
          io::stdin().read_line(&mut port).unwrap();
          port = port.trim().to_string();
          if !port.is_empty() {
            opts = opts.port(port.parse()?);
          }
        }

        if let Some(database) = args.database {
          opts = opts.database(&database);
        } else {
          let mut database: String = String::new();
          print!("database (ex. postgres): ");
          io::stdout().flush().unwrap();
          io::stdin().read_line(&mut database).unwrap();
          database = database.trim().to_string();
          if !database.is_empty() {
            opts = opts.database(&database);
          }
        }

        Ok(opts)
      },
    }
  }
}

async fn query_with_pool(pool: Arc<sqlx::Pool<Postgres>>, query: String) -> Result<Rows> {
  query_with_stream(&*pool.clone(), &query).await
}

async fn query_with_conn(conn: &mut PgConnection, query: String) -> Result<Rows> {
  query_with_stream(conn, &query).await
}

async fn query_with_stream<'a, E>(e: E, query: &'a str) -> Result<Rows>
where
  E: sqlx::Executor<'a, Database = sqlx::Postgres>,
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
  mut tx: PostgresTransaction<'static>,
  query: &str,
) -> (Result<Either<u64, Rows>>, PostgresTransaction<'static>)
where
  for<'c> <sqlx::Postgres as sqlx::Database>::Arguments<'c>: sqlx::IntoArguments<'c, sqlx::Postgres>,
  for<'c> &'c mut <sqlx::Postgres as sqlx::Database>::Connection: sqlx::Executor<'c, Database = sqlx::Postgres>,
{
  let first_query = super::get_first_query(query.to_string(), Driver::Postgres);
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

fn get_headers(row: &<sqlx::Postgres as sqlx::Database>::Row) -> Headers {
  row
    .columns()
    .iter()
    .map(|col| Header { name: col.name().to_string(), type_name: col.type_info().to_string() })
    .collect()
}

fn row_to_vec(row: &<sqlx::Postgres as sqlx::Database>::Row) -> Vec<String> {
  row.columns().iter().map(|col| parse_value(row, col).unwrap().string).collect()
}

// parsed based on https://docs.rs/sqlx/latest/sqlx/postgres/types/index.html
fn parse_value(row: &<Postgres as sqlx::Database>::Row, col: &<Postgres as sqlx::Database>::Column) -> Option<Value> {
  let col_type = col.type_info().to_string();
  if row.try_get_raw(col.ordinal()).is_ok_and(|v| v.is_null()) {
    return Some(Value { parse_error: false, string: "NULL".to_string(), is_null: true });
  }
  match col_type.to_uppercase().as_str() {
    "TIMESTAMPTZ" => Some(row.try_get::<chrono::DateTime<chrono::Utc>, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "TIMESTAMP" => Some(row.try_get::<chrono::NaiveDateTime, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "DATE" => Some(row.try_get::<chrono::NaiveDate, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "TIME" => Some(row.try_get::<chrono::NaiveTime, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "UUID" => Some(row.try_get::<Uuid, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "INET" | "CIDR" => Some(row.try_get::<std::net::IpAddr, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "JSON" | "JSONB" => Some(row.try_get::<serde_json::Value, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "BOOL" => Some(row.try_get::<bool, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "SMALLINT" | "SMALLSERIAL" | "INT2" => Some(row.try_get::<i16, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "INT" | "SERIAL" | "INT4" => Some(row.try_get::<i32, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "BIGINT" | "BIGSERIAL" | "INT8" => Some(row.try_get::<i64, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "REAL" | "FLOAT4" => Some(row.try_get::<f32, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "DOUBLE PRECISION" | "FLOAT8" => Some(row.try_get::<f64, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value { parse_error: false, string: received.to_string(), is_null: false },
    )),
    "TEXT" | "VARCHAR" | "NAME" | "CITEXT" | "BPCHAR" | "CHAR" => {
      Some(row.try_get::<String, usize>(col.ordinal()).map_or(
        Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
        |received| Value { parse_error: false, string: received.to_string(), is_null: false },
      ))
    },
    "BYTEA" => Some(row.try_get::<Vec<u8>, usize>(col.ordinal()).map_or(
      Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
      |received| Value {
        parse_error: false,
        string: received.iter().fold(String::new(), |mut output, b| {
          let _ = write!(output, "{b:02X}");
          output
        }),
        is_null: false,
      },
    )),
    "VOID" => Some(Value { parse_error: false, string: "".to_string(), is_null: false }),
    _ if col_type.to_uppercase().ends_with("[]") => {
      let array_type = col_type.to_uppercase().replace("[]", "");
      match array_type.as_str() {
        "TIMESTAMPTZ" => Some(row.try_get::<Vec<chrono::DateTime<chrono::Utc>>, usize>(col.ordinal()).map_or(
          Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
          |received| Value { parse_error: false, string: vec_to_string(received), is_null: false },
        )),
        "TIMESTAMP" => Some(row.try_get::<Vec<chrono::NaiveDateTime>, usize>(col.ordinal()).map_or(
          Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
          |received| Value { parse_error: false, string: vec_to_string(received), is_null: false },
        )),
        "DATE" => Some(row.try_get::<Vec<chrono::NaiveDate>, usize>(col.ordinal()).map_or(
          Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
          |received| Value { parse_error: false, string: vec_to_string(received), is_null: false },
        )),
        "TIME" => Some(row.try_get::<Vec<chrono::NaiveTime>, usize>(col.ordinal()).map_or(
          Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
          |received| Value { parse_error: false, string: vec_to_string(received), is_null: false },
        )),
        "UUID" => Some(row.try_get::<Vec<Uuid>, usize>(col.ordinal()).map_or(
          Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
          |received| Value { parse_error: false, string: vec_to_string(received), is_null: false },
        )),
        "INET" | "CIDR" => Some(row.try_get::<Vec<std::net::IpAddr>, usize>(col.ordinal()).map_or(
          Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
          |received| Value { parse_error: false, string: vec_to_string(received), is_null: false },
        )),
        "JSON" | "JSONB" => Some(row.try_get::<Vec<serde_json::Value>, usize>(col.ordinal()).map_or(
          Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
          |received| Value { parse_error: false, string: vec_to_string(received), is_null: false },
        )),
        "BOOL" => Some(row.try_get::<Vec<bool>, usize>(col.ordinal()).map_or(
          Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
          |received| Value { parse_error: false, string: vec_to_string(received), is_null: false },
        )),
        "SMALLINT" | "SMALLSERIAL" | "INT2" => Some(row.try_get::<Vec<i16>, usize>(col.ordinal()).map_or(
          Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
          |received| Value { parse_error: false, string: vec_to_string(received), is_null: false },
        )),
        "INT" | "SERIAL" | "INT4" => Some(row.try_get::<Vec<i32>, usize>(col.ordinal()).map_or(
          Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
          |received| Value { parse_error: false, string: vec_to_string(received), is_null: false },
        )),
        "BIGINT" | "BIGSERIAL" | "INT8" => Some(row.try_get::<Vec<i64>, usize>(col.ordinal()).map_or(
          Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
          |received| Value { parse_error: false, string: vec_to_string(received), is_null: false },
        )),
        "REAL" | "FLOAT4" => Some(row.try_get::<Vec<f32>, usize>(col.ordinal()).map_or(
          Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
          |received| Value { parse_error: false, string: vec_to_string(received), is_null: false },
        )),
        "DOUBLE PRECISION" | "FLOAT8" => Some(row.try_get::<Vec<f64>, usize>(col.ordinal()).map_or(
          Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
          |received| Value { parse_error: false, string: vec_to_string(received), is_null: false },
        )),
        "TEXT" | "VARCHAR" | "NAME" | "CITEXT" | "BPCHAR" | "CHAR" => {
          Some(row.try_get::<Vec<String>, usize>(col.ordinal()).map_or(
            Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
            |received| Value { parse_error: false, string: vec_to_string(received), is_null: false },
          ))
        },
        "BYTEA" => Some(row.try_get::<Vec<u8>, usize>(col.ordinal()).map_or(
          Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
          |received| Value {
            parse_error: false,
            string: received.iter().fold(String::new(), |mut output, b| {
              let _ = write!(output, "{b:02X}");
              output
            }),
            is_null: false,
          },
        )),
        _ => {
          // try to cast custom or other types to strings
          Some(row.try_get_unchecked::<Vec<String>, usize>(col.ordinal()).map_or(
            Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
            |received| Value { parse_error: false, string: vec_to_string(received), is_null: false },
          ))
        },
      }
    },
    _ => {
      // try to cast custom or other types to strings
      Some(row.try_get_unchecked::<String, usize>(col.ordinal()).map_or(
        Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
        |received| Value { parse_error: false, string: received.to_string(), is_null: false },
      ))
    },
  }
}

#[cfg(test)]
mod tests {
  use sqlparser::{dialect::PostgreSqlDialect, parser::ParserError};

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
        "select * from \"public\".\"users\"",
        Ok(("SELECT * FROM \"public\".\"users\"".to_owned(), Box::new(|s| matches!(s, Statement::Query(_))))),
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
      (
        "-- select blah;\nselect * from users\n-- insert blah",
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
        Ok(("DELETE FROM users WHERE id = 1".to_owned(), Box::new(|s| matches!(s, Statement::Delete(_))))),
      ),
      // drop
      ("DROP TABLE users", Ok(("DROP TABLE users".to_owned(), Box::new(|s| matches!(s, Statement::Drop { .. }))))),
      // explain
      (
        "EXPLAIN SELECT * FROM users",
        Ok(("EXPLAIN SELECT * FROM users".to_owned(), Box::new(|s| matches!(s, Statement::Explain { .. })))),
      ),
    ];

    let dialect = Box::new(PostgreSqlDialect {});

    for (input, expected_output) in test_cases {
      let result = get_first_query(input.to_string(), Driver::Postgres);
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
  fn test_execution_type_postgres() {
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
        get_execution_type(query.to_string(), false, Driver::Postgres).unwrap().0,
        expected,
        "Failed for query: {query}"
      );
    }
  }
}
