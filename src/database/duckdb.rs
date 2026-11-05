use std::{
  fmt::Write,
  io::{self, Write as _},
  string::String,
  sync::atomic::AtomicBool,
};

use async_trait::async_trait;
use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, NaiveTime};
use color_eyre::eyre::{self, Result};
use duckdb::{
  Connection,
  types::{OrderedMap, TimeUnit, Value as DuckValue},
};
use tracing::Instrument;

mod worker;

use worker::{Request, Worker, check_cancelled};

use crate::cli::{Cli, Driver};

use super::{
  Database, DbTaskResult, Header, Headers, QueryResultsWithMetadata, Rows, builtin_functions,
};
use crate::completion::{TableColumns, TableRef, table_columns_from_rows};

struct DuckDbTask {
  request: Request<Rows>,
  statement_type: Option<sqlparser::ast::Statement>,
}

#[derive(Default)]
pub struct DuckDbDriver {
  worker: Option<Worker>,
  task: Option<DuckDbTask>,
}

#[async_trait(?Send)]
impl Database for DuckDbDriver {
  fn builtin_functions(&self) -> &'static [&'static str] {
    builtin_functions::DUCKDB
  }

  async fn init(&mut self, args: Cli) -> Result<String> {
    let path = Self::build_connection_path(args)?;
    self.abort_query().await?;
    self.worker = Some(Worker::open(path.clone()).await?);
    Ok(path)
  }

  async fn start_query(&mut self, query: String, bypass_parser: bool) -> Result<()> {
    let (first_query, statement_type) = match bypass_parser {
      true => (query, None),
      false => {
        let (first, stmt) = super::get_first_query(query, Driver::DuckDb)?;
        (first, Some(stmt))
      },
    };
    self.abort_query().await?;
    let request = self
      .worker()?
      .submit(move |connection, cancelled| run_query(connection, &first_query, cancelled))?;
    self.task = Some(DuckDbTask { request, statement_type });
    Ok(())
  }

  async fn abort_query(&mut self) -> Result<bool> {
    let Some(task) = self.task.as_mut() else { return Ok(false) };
    // TODO: Avoid awaiting native cancellation on the UI/quit path. Blocking I/O, such as
    // read_csv on a named pipe awaiting a writer, may not observe interrupts, leaving this
    // wait stuck indefinitely and preventing UI input and terminal cleanup.
    let result = task.request.cancel_and_wait().await;
    self.task = None;
    result?;
    Ok(true)
  }

  async fn get_query_results(&mut self) -> Result<DbTaskResult> {
    let Some(task) = self.task.as_mut() else { return Ok(DbTaskResult::NoTask) };
    match task.request.try_result() {
      Ok(Some(rows)) => {
        let task = self.task.take().unwrap();
        Ok(DbTaskResult::Finished(QueryResultsWithMetadata::new(rows, task.statement_type)))
      },
      Ok(None) => Ok(DbTaskResult::Pending),
      Err(error) => {
        self.task = None;
        Err(error)
      },
    }
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

  fn start_load_menu(&self) -> Result<tokio::task::JoinHandle<Result<Rows>>> {
    let request = self.worker()?.submit(move |connection, cancelled| {
      run_query(
        connection,
        "select table_schema,
        table_name,
        case
          when table_type = 'BASE TABLE' then 'table'
          when table_type = 'VIEW' then 'view'
          else 'table'
        end as object_kind
      from information_schema.tables
      where table_schema != 'information_schema'
      group by table_schema, table_name, table_type
      order by table_schema, object_kind, table_name asc",
        cancelled,
      )
    })?;
    Ok(tokio::spawn(request.wait().in_current_span()))
  }

  fn start_load_columns(
    &self,
    tables: Vec<TableRef>,
  ) -> Result<tokio::task::JoinHandle<Result<Vec<TableColumns>>>> {
    let request = self.worker()?.submit(move |connection, cancelled| {
      let mut output = Vec::with_capacity(tables.len());
      for table in tables {
        let schema = table.schema.replace('\'', "''");
        let name = table.table.replace('\'', "''");
        let query = format!(
          "select column_name, data_type from information_schema.columns where table_schema = '{schema}' and table_name = '{name}' order by ordinal_position"
        );
        output.push(table_columns_from_rows(table, run_query(connection, &query, cancelled)?));
      }
      Ok(output)
    })?;
    Ok(tokio::spawn(request.wait().in_current_span()))
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
    format!(
      "select indexname, indexdef, * from pg_indexes where schemaname = '{}' and tablename = '{}'",
      schema, table
    )
  }

  fn preview_policies_query(&self, schema: &str, table: &str) -> String {
    "select 'DuckDB does not support row-level security policies' as message".to_owned()
  }

  fn preview_view_definition_query(&self, schema: &str, view: &str, materialized: bool) -> String {
    if materialized {
      return "select 'DuckDB does not support materialized views' as message".to_owned();
    }
    format!(
      "select view_definition
        from information_schema.views
        where table_schema = '{schema}' and table_name = '{view}'"
    )
  }
}

fn run_query(connection: &Connection, query: &str, cancelled: &AtomicBool) -> Result<Rows> {
  check_cancelled(cancelled)?;
  let mut statement = connection.prepare(query)?;
  check_cancelled(cancelled)?;
  let rows = statement.query([])?;
  fetch_rows(rows, cancelled)
}

fn fetch_rows(mut rows: duckdb::Rows<'_>, cancelled: &AtomicBool) -> Result<Rows> {
  let mut headers: Headers = Vec::new();
  let mut results: Vec<Vec<String>> = Vec::new();
  while let Some(row) = rows.next()? {
    check_cancelled(cancelled)?;
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
      let value = row.get::<usize, DuckValue>(i);
      match value {
        Ok(value) => r.push(duck_value_to_string(&value)),
        Err(_) => r.push("_ERROR_".to_string()),
      }
    }
    results.push(r);
  }
  Ok(Rows { headers, rows: results, rows_affected: None })
}

fn duck_value_to_string(value: &DuckValue) -> String {
  match value {
    DuckValue::Null => "NULL".to_string(),
    DuckValue::Boolean(v) => v.to_string(),
    DuckValue::TinyInt(v) => v.to_string(),
    DuckValue::SmallInt(v) => v.to_string(),
    DuckValue::Int(v) => v.to_string(),
    DuckValue::BigInt(v) => v.to_string(),
    DuckValue::HugeInt(v) => v.to_string(),
    DuckValue::UHugeInt(v) => v.to_string(),
    DuckValue::UTinyInt(v) => v.to_string(),
    DuckValue::USmallInt(v) => v.to_string(),
    DuckValue::UInt(v) => v.to_string(),
    DuckValue::UBigInt(v) => v.to_string(),
    DuckValue::Float(v) => v.to_string(),
    DuckValue::Double(v) => v.to_string(),
    DuckValue::Decimal(v) => v.to_string(),
    DuckValue::Timestamp(unit, raw) => format_timestamp(*unit, *raw),
    DuckValue::Text(text) => text.clone(),
    DuckValue::Blob(bytes) | DuckValue::Geometry(bytes) => bytes_to_string(bytes),
    DuckValue::Date32(days) => format_date(*days),
    DuckValue::Time64(unit, raw) => format_time(*unit, *raw),
    DuckValue::Interval { months, days, nanos } => format_interval(*months, *days, *nanos),
    DuckValue::List(values) | DuckValue::Array(values) => format_list(values),
    DuckValue::Enum(value) => value.clone(),
    DuckValue::Struct(map) => format_struct(map),
    DuckValue::Map(map) => format_map(map),
    DuckValue::Union(inner) => duck_value_to_string(inner.as_ref()),
    _ => format!("{value:?}"),
  }
}

fn format_timestamp(unit: TimeUnit, raw: i64) -> String {
  match unit {
    TimeUnit::Second => format_timestamp_from_parts(raw, 0),
    TimeUnit::Millisecond => {
      let secs = raw.div_euclid(1_000);
      let nanos = (raw.rem_euclid(1_000) * 1_000_000) as u32;
      format_timestamp_from_parts(secs, nanos)
    },
    TimeUnit::Microsecond => {
      let secs = raw.div_euclid(1_000_000);
      let nanos = (raw.rem_euclid(1_000_000) * 1_000) as u32;
      format_timestamp_from_parts(secs, nanos)
    },
    TimeUnit::Nanosecond => {
      let secs = raw.div_euclid(1_000_000_000);
      let nanos = raw.rem_euclid(1_000_000_000) as u32;
      format_timestamp_from_parts(secs, nanos)
    },
  }
}

fn format_timestamp_from_parts(secs: i64, nanos: u32) -> String {
  DateTime::from_timestamp(secs, nanos)
    .map_or_else(|| format!("{secs}.{nanos:09}"), |dt| dt.to_string())
}

fn format_date(days_since_epoch: i32) -> String {
  let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
  epoch
    .checked_add_signed(ChronoDuration::days(days_since_epoch as i64))
    .map_or_else(|| days_since_epoch.to_string(), |date| date.to_string())
}

fn format_time(unit: TimeUnit, raw: i64) -> String {
  let nanos_total: i128 = match unit {
    TimeUnit::Second => (raw as i128) * 1_000_000_000,
    TimeUnit::Millisecond => (raw as i128) * 1_000_000,
    TimeUnit::Microsecond => (raw as i128) * 1_000,
    TimeUnit::Nanosecond => raw as i128,
  };
  let nanos_per_day = 86_400_000_000_000i128;
  let normalized = ((nanos_total % nanos_per_day) + nanos_per_day) % nanos_per_day;
  let secs = (normalized / 1_000_000_000) as u32;
  let nanos = (normalized % 1_000_000_000) as u32;
  NaiveTime::from_num_seconds_from_midnight_opt(secs, nanos)
    .map(|time| time.to_string())
    .unwrap_or_else(|| raw.to_string())
}

fn format_interval(months: i32, days: i32, nanos: i64) -> String {
  format!("months={months}, days={days}, nanos={nanos}")
}

fn format_list(values: &[DuckValue]) -> String {
  let formatted: Vec<String> = values.iter().map(duck_value_to_string).collect();
  format!("[{}]", formatted.join(", "))
}

fn format_struct(map: &OrderedMap<String, DuckValue>) -> String {
  let formatted: Vec<String> =
    map.iter().map(|(key, value)| format!("{key}: {}", duck_value_to_string(value))).collect();
  format!("{{{}}}", formatted.join(", "))
}

fn format_map(map: &OrderedMap<DuckValue, DuckValue>) -> String {
  let formatted: Vec<String> = map
    .iter()
    .map(|(key, value)| format!("{}: {}", duck_value_to_string(key), duck_value_to_string(value)))
    .collect();
  format!("{{{}}}", formatted.join(", "))
}

fn bytes_to_string(bytes: &[u8]) -> String {
  match std::str::from_utf8(bytes) {
    Ok(text) => text.to_owned(),
    Err(_) => {
      let mut output = String::from("0x");
      for b in bytes {
        let _ = write!(&mut output, "{b:02X}");
      }
      output
    },
  }
}

impl DuckDbDriver {
  pub fn new() -> Self {
    Self::default()
  }

  fn worker(&self) -> Result<&Worker> {
    self.worker.as_ref().ok_or_else(|| eyre::eyre!("DuckDB connection is not initialized"))
  }

  fn build_connection_path(args: crate::cli::Cli) -> Result<String> {
    match args.connection_url {
      Some(url) => Ok(url.trim().trim_start_matches("jdbc:").to_string()),
      None => {
        if let Some(database) = args.database {
          Ok(database)
        } else {
          let mut database = String::new();
          print!("database file path (or ':memory:'): ");
          io::stdout().flush()?;
          io::stdin().read_line(&mut database)?;
          let database = database.trim().to_string();
          if database.is_empty() {
            Err(eyre::Report::msg("Database file path is required"))
          } else {
            Ok(database)
          }
        }
      },
    }
  }
}

#[cfg(test)]
mod tests {

  use std::time::Duration;

  use clap::Parser;
  use tokio::time::{sleep, timeout};

  use super::*;

  const LONG_QUERY: &str = "SELECT count(*) FROM range(10000000) t1, range(1000000) t2";

  async fn memory_driver() -> DuckDbDriver {
    let mut driver = DuckDbDriver::new();
    driver.init(Cli::parse_from(["rainfrog", "--database", ":memory:"])).await.unwrap();
    driver
  }

  async fn query_result(driver: &mut DuckDbDriver) -> QueryResultsWithMetadata {
    timeout(Duration::from_secs(5), async {
      loop {
        match driver.get_query_results().await.unwrap() {
          DbTaskResult::Finished(result) => break result,
          DbTaskResult::Pending => sleep(Duration::from_millis(1)).await,
          _ => panic!("expected pending or finished DuckDB query"),
        }
      }
    })
    .await
    .expect("DuckDB query should finish")
  }

  async fn execute(driver: &mut DuckDbDriver, query: &str) -> Rows {
    driver.start_query(query.to_owned(), false).await.unwrap();
    query_result(driver).await.results.unwrap()
  }

  async fn track_progress(driver: &DuckDbDriver) -> duckdb::ProgressHandle {
    let request = driver
      .worker()
      .unwrap()
      .submit(|connection, _| {
        connection.set_query_progress_tracking(true)?;
        connection.execute_batch("SET threads = 1")?;
        Ok(connection.progress_handle())
      })
      .unwrap();
    request.wait().await.unwrap()
  }

  async fn wait_for_execution(progress: &duckdb::ProgressHandle) {
    timeout(Duration::from_secs(5), async {
      while progress.query_progress().total_rows_to_process() == 0 {
        sleep(Duration::from_millis(1)).await;
      }
    })
    .await
    .expect("the native DuckDB query should start without blocking the async runtime");
  }

  #[tokio::test]
  async fn query_workflow_displays_wide_numbers_and_geometry() {
    let mut driver = memory_driver().await;

    let queries = [
      "CREATE TABLE samples AS SELECT \
       '340282366920938463463374607431768211455'::UHUGEINT AS huge, \
       '123456789012345678901234567890.12345678'::DECIMAL(38, 8) AS amount, \
       from_hex('0101000000000000000000F03F0000000000000040')::GEOMETRY AS point, \
       [1, NULL, 3] AS items",
      "SELECT huge, amount, point, items FROM samples",
    ];

    for (index, query) in queries.iter().enumerate() {
      driver.start_query((*query).to_owned(), false).await.unwrap();
      let result = query_result(&mut driver).await;
      assert!(result.statement_type.is_some());
      let rows = result.results.unwrap();
      if index == 1 {
        assert_eq!(
          rows.headers.iter().map(|header| header.name.as_str()).collect::<Vec<_>>(),
          vec!["huge", "amount", "point", "items"]
        );
        assert_eq!(
          rows.rows,
          vec![vec![
            "340282366920938463463374607431768211455",
            "123456789012345678901234567890.12345678",
            "0x0101000000000000000000F03F0000000000000040",
            "[1, NULL, 3]",
          ]]
        );
      }
    }
  }

  #[tokio::test]
  async fn queries_and_completion_share_temporary_tables_and_session_state() {
    let mut driver = memory_driver().await;
    execute(&mut driver, "CREATE TEMP TABLE users(id integer, name varchar)").await;
    execute(&mut driver, "INSERT INTO users VALUES (42, 'kept')").await;
    driver.start_query("SET VARIABLE session_value = 'session kept'".into(), true).await.unwrap();
    query_result(&mut driver).await.results.unwrap();

    let menu = driver.start_load_menu().unwrap().await.unwrap().unwrap();
    let user_row = menu.rows.iter().find(|row| row.get(1).is_some_and(|name| name == "users"));
    assert!(user_row.is_some());

    let table = TableRef { schema: "main".into(), table: "users".into() };
    let columns = driver.start_load_columns(vec![table]).unwrap().await.unwrap().unwrap();
    assert_eq!(
      columns[0].columns.iter().map(|column| column.name.as_str()).collect::<Vec<_>>(),
      vec!["id", "name"]
    );
    assert_eq!(
      execute(&mut driver, "SELECT id, name, getvariable('session_value') FROM users").await.rows,
      vec![vec!["42", "kept", "session kept"]]
    );
  }

  #[tokio::test]
  async fn bypass_parser_sends_raw_query_to_duckdb() {
    let mut driver = memory_driver().await;

    driver.start_query("SELEC 1".to_owned(), true).await.unwrap();

    let result = query_result(&mut driver).await;
    assert!(result.results.is_err());
    assert!(result.statement_type.is_none());
    assert_eq!(execute(&mut driver, "SELECT 42").await.rows, vec![vec!["42"]]);
  }

  #[tokio::test]
  async fn cancellation_interrupts_native_work_and_preserves_the_session() {
    let mut driver = memory_driver().await;
    execute(&mut driver, "CREATE TEMP TABLE kept AS SELECT 42 AS id").await;
    let progress = track_progress(&driver).await;

    for _ in 0..2 {
      driver.start_query(LONG_QUERY.to_owned(), false).await.unwrap();
      wait_for_execution(&progress).await;
      let menu = driver.start_load_menu().unwrap();

      assert!(timeout(Duration::from_secs(5), driver.abort_query()).await.unwrap().unwrap());
      assert!(matches!(driver.get_query_results().await.unwrap(), DbTaskResult::NoTask));
      let menu = timeout(Duration::from_secs(5), menu).await.unwrap().unwrap().unwrap();
      assert!(menu.rows.iter().any(|row| row.get(1).is_some_and(|name| name == "kept")));
      assert_eq!(execute(&mut driver, "SELECT id FROM kept").await.rows, vec![vec!["42"]]);
      assert!(!driver.abort_query().await.unwrap());
    }

    // Starting a replacement query must also stop native work before reusing the session.
    driver.start_query(LONG_QUERY.to_owned(), false).await.unwrap();
    wait_for_execution(&progress).await;
    timeout(Duration::from_secs(5), driver.start_query("SELECT id FROM kept".into(), false))
      .await
      .unwrap()
      .unwrap();
    assert_eq!(query_result(&mut driver).await.results.unwrap().rows, vec![vec!["42"]]);
  }

  #[tokio::test]
  async fn cancelling_a_queued_write_skips_its_side_effects() {
    let mut driver = memory_driver().await;
    execute(&mut driver, "CREATE TEMP TABLE kept(id integer)").await;
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let preceding = driver
      .worker()
      .unwrap()
      .submit(move |connection, cancelled| {
        ready_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(5))?;
        run_query(connection, "SELECT 123", cancelled)
      })
      .unwrap();
    ready_rx.await.unwrap();
    driver.start_query("INSERT INTO kept VALUES (1)".into(), false).await.unwrap();

    let (aborted, ()) = timeout(Duration::from_secs(5), async {
      tokio::join!(biased; driver.abort_query(), async {
        // Poll cancellation before releasing the worker so the write stays queued.
        release_tx.send(()).unwrap();
      })
    })
    .await
    .unwrap();
    assert!(aborted.unwrap());
    assert_eq!(preceding.wait().await.unwrap().rows, vec![vec!["123"]]);
    assert_eq!(execute(&mut driver, "SELECT count(*) FROM kept").await.rows, vec![vec!["0"]]);
  }

  #[tokio::test]
  async fn cancellation_during_preparation_still_interrupts_execution() {
    let mut driver = memory_driver().await;
    let (prepared_tx, prepared_rx) = tokio::sync::oneshot::channel();
    let (execute_tx, execute_rx) = std::sync::mpsc::channel();
    let request = driver
      .worker()
      .unwrap()
      .submit(move |connection, cancelled| {
        let mut statement = connection.prepare(LONG_QUERY)?;
        prepared_tx.send(()).unwrap();
        execute_rx.recv_timeout(Duration::from_secs(5))?;
        // Deliberately enter execution after the first interrupt, exercising DuckDB's reset.
        fetch_rows(statement.query([])?, cancelled)
      })
      .unwrap();
    driver.task = Some(DuckDbTask { request, statement_type: None });
    prepared_rx.await.unwrap();

    let (aborted, ()) = timeout(Duration::from_secs(5), async {
      tokio::join!(biased; driver.abort_query(), async {
        execute_tx.send(()).unwrap();
      })
    })
    .await
    .unwrap();
    assert!(aborted.unwrap());
    assert_eq!(execute(&mut driver, "SELECT 42").await.rows, vec![vec!["42"]]);
  }

  #[tokio::test]
  async fn dropping_the_driver_stops_native_work_and_closes_the_connection() {
    let mut driver = memory_driver().await;
    let progress = track_progress(&driver).await;
    driver.start_query(LONG_QUERY.to_owned(), false).await.unwrap();
    wait_for_execution(&progress).await;
    let menu = driver.start_load_menu().unwrap();

    drop(driver);

    timeout(Duration::from_secs(5), async {
      while progress.is_connected() {
        sleep(Duration::from_millis(1)).await;
      }
    })
    .await
    .expect("dropping the driver should close the worker's connection");
    assert!(timeout(Duration::from_secs(5), menu).await.unwrap().unwrap().is_err());
  }
}
