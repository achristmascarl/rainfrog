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
  Column, Either, MySqlConnection, Row, ValueRef,
  mysql::{MySql, MySqlConnectOptions, MySqlPoolOptions, MySqlSslMode},
  pool::PoolConnection,
};
use tokio::{sync::Mutex, task::JoinHandle};
use tracing::Instrument;

use super::{
  Database, DbTaskResult, Driver, Header, Headers, QueryResultsWithMetadata, QueryTask, Rows,
  Value, builtin_functions,
};
use crate::completion::{TableColumns, TableRef, table_columns_from_rows};

type MySqlTransaction = sqlx::Transaction<'static, MySql>;
type ConnectionTask = JoinHandle<Result<(Arc<Mutex<PoolConnection<MySql>>>, u64, bool)>>;
type TransactionAcquireTask = JoinHandle<Result<(MySqlTransaction, u64)>>;
type TransactionTask = JoinHandle<(QueryResultsWithMetadata, MySqlTransaction)>;
enum MySqlTask {
  QueryConnect {
    handle: ConnectionTask,
    first_query: String,
    statement_type: Option<Statement>,
  },
  Query(QueryTask),
  TxConnect {
    handle: TransactionAcquireTask,
    first_query: String,
    statement_type: Statement,
    display_statement_type: Box<Statement>,
  },
  TxStart(TransactionTask),
  TxPending(Box<(MySqlTransaction, QueryResultsWithMetadata)>),
}

const MENU_QUERY: &str =
  "select menu_items.table_schema, menu_items.object_name, menu_items.object_kind
      from (
        select table_schema as table_schema,
          table_name as object_name,
          case
            when table_type = 'BASE TABLE' then 'table'
            when table_type = 'VIEW' then 'view'
            else 'table'
          end as object_kind
        from information_schema.tables
        where table_schema not in ('mysql', 'information_schema', 'performance_schema', 'sys')
        union all
        select routine_schema as table_schema,
          routine_name as object_name,
          'function' as object_kind
        from information_schema.routines
        where routine_type = 'FUNCTION'
          and routine_schema not in ('mysql', 'information_schema', 'performance_schema', 'sys')
      ) menu_items
      order by menu_items.table_schema, menu_items.object_kind, menu_items.object_name asc";

const SIMPLE_MENU_QUERY: &str = "select table_schema, table_name as object_name,
        case
          when table_type in ('VIEW', 'MATERIALIZED VIEW') then 'view'
          else 'table'
        end as object_kind
      from information_schema.tables
      where table_schema not in ('mysql', 'information_schema', 'performance_schema', 'sys')
      order by table_schema, object_kind, object_name asc";

#[derive(Default)]
pub struct MySqlDriver {
  pool: Option<Arc<sqlx::Pool<MySql>>>,
  task: Option<MySqlTask>,
  querying_conn: Option<Arc<Mutex<PoolConnection<MySql>>>>,
  querying_pid: Option<String>,
}

#[async_trait(?Send)]
impl Database for MySqlDriver {
  fn builtin_functions(&self) -> &'static [&'static str] {
    builtin_functions::MYSQL
  }

  async fn init(&mut self, args: crate::cli::Cli) -> Result<String> {
    let opts = super::mysql::MySqlDriver::build_connection_opts(args)?;
    let pool =
      Arc::new(MySqlPoolOptions::new().max_connections(3).connect_with(opts.clone()).await?);
    self.pool = Some(pool);
    Ok(format!("{}/{}", opts.get_port(), opts.get_database().unwrap_or("mysql")))
  }

  // since it's possible for raw_sql to execute multiple queries in a single string,
  // we only execute the first one and then drop the rest.
  async fn start_query(&mut self, query: String, bypass_parser: bool) -> Result<()> {
    let (first_query, statement_type) = match bypass_parser {
      true => (query, None),
      false => {
        let (first, stmt) = super::get_first_query(query, Driver::MySql)?;
        (first, Some(stmt))
      },
    };
    let pool = self.pool.clone().unwrap();
    self.task = Some(MySqlTask::QueryConnect {
      handle: tokio::spawn(
        async move {
          let conn = Arc::new(Mutex::new(pool.acquire().await?));
          let mut guard = conn.lock().await;
          let pid = connection_pid(guard.as_mut()).await?;
          let no_backslash_escapes =
            if bypass_parser { session_no_backslash_escapes(guard.as_mut()).await? } else { false };
          drop(guard);
          Ok((conn, pid, no_backslash_escapes))
        }
        .in_current_span(),
      ),
      first_query,
      statement_type,
    });
    Ok(())
  }

  async fn abort_query(&mut self) -> Result<bool> {
    match self.task.take() {
      Some(task) => {
        match task {
          MySqlTask::QueryConnect { handle, .. } => handle.abort(),
          MySqlTask::Query(handle) => handle.abort(),
          MySqlTask::TxConnect { handle, .. } => handle.abort(),
          MySqlTask::TxStart(handle) => handle.abort(),
          _ => {},
        };
        if let Some(pid) = self.querying_pid.take() {
          let result =
            sqlx::raw_sql(&format!("KILL {pid}")).execute(&*self.pool.clone().unwrap()).await;
          let msg = match result {
            Ok(_) => "Successfully killed".to_string(),
            Err(e) => format!("Failed to kill: {e:?}"),
          };
          log::info!("Tried to cancel backend process with PID {pid}: {msg} ");
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
      Some(MySqlTask::QueryConnect { handle, first_query, statement_type }) => {
        if !handle.is_finished() {
          (
            DbTaskResult::Pending,
            Some(MySqlTask::QueryConnect { handle, first_query, statement_type }),
          )
        } else {
          match handle.await? {
            Ok((conn, pid, no_backslash_escapes)) => {
              log::info!("Starting query with PID {}", pid.clone());
              self.querying_conn = Some(conn.clone());
              self.querying_pid = Some(pid.to_string());
              let first_query = if statement_type.is_none() {
                preprocess_delimiter_script(first_query, no_backslash_escapes)
              } else {
                first_query
              };
              (
                DbTaskResult::Pending,
                Some(MySqlTask::Query(spawn_query_task(conn, first_query, statement_type))),
              )
            },
            Err(e) => {
              log::error!("Connection acquisition failed: {e:?}");
              (DbTaskResult::Finished(QueryResultsWithMetadata::new(Err(e), statement_type)), None)
            },
          }
        }
      },
      Some(MySqlTask::Query(handle)) => {
        if !handle.is_finished() {
          (DbTaskResult::Pending, Some(MySqlTask::Query(handle)))
        } else {
          let result = handle.await?;
          self.querying_conn = None;
          self.querying_pid = None;
          (DbTaskResult::Finished(result), None)
        }
      },
      Some(MySqlTask::TxConnect {
        handle,
        first_query,
        statement_type,
        display_statement_type,
      }) => {
        if !handle.is_finished() {
          (
            DbTaskResult::Pending,
            Some(MySqlTask::TxConnect {
              handle,
              first_query,
              statement_type,
              display_statement_type,
            }),
          )
        } else {
          match handle.await? {
            Ok((tx, pid)) => {
              log::info!("Starting transaction with PID {}", pid.clone());
              self.querying_pid = Some(pid.to_string());
              (
                DbTaskResult::Pending,
                Some(MySqlTask::TxStart(spawn_tx_task(
                  tx,
                  first_query,
                  statement_type,
                  *display_statement_type,
                ))),
              )
            },
            Err(e) => {
              log::error!("Transaction didn't start: {e:?}");
              (
                DbTaskResult::Finished(QueryResultsWithMetadata::with_display_statement_type(
                  Err(e),
                  Some(statement_type),
                  Some(*display_statement_type),
                )),
                None,
              )
            },
          }
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
          match result {
            // if tx failed to start, return the error immediately
            QueryResultsWithMetadata {
              results: Err(e),
              statement_type,
              display_statement_type,
            } => {
              log::error!("Transaction didn't start: {e:?}");
              self.querying_conn = None;
              self.querying_pid = None;
              (
                DbTaskResult::Finished(QueryResultsWithMetadata::with_display_statement_type(
                  Err(e),
                  statement_type,
                  display_statement_type,
                )),
                None,
              )
            },
            _ => (
              DbTaskResult::ConfirmTx(
                rows_affected,
                result.statement_type.clone(),
                result.display_statement_type.clone(),
              ),
              Some(MySqlTask::TxPending(Box::new((tx, result)))),
            ),
          }
        }
      },
      Some(MySqlTask::TxPending(b)) => (DbTaskResult::Pending, Some(MySqlTask::TxPending(b))),
    };
    self.task = next_task;
    Ok(task_result)
  }

  async fn start_tx(&mut self, query: String) -> Result<()> {
    let (first_query, statement_type) = super::get_first_query(query, Driver::MySql)?;
    let display_statement_type = super::get_display_statement_for_execution_type(&statement_type);
    let statement_type = super::get_statement_for_execution_type(&statement_type);
    let pool = self.pool.clone().unwrap();
    self.task = Some(MySqlTask::TxConnect {
      handle: tokio::spawn(
        async move {
          let mut tx = pool.begin().await?;
          let pid = connection_pid(&mut tx).await?;
          Ok((tx, pid))
        }
        .in_current_span(),
      ),
      first_query,
      statement_type,
      display_statement_type: Box::new(display_statement_type),
    });
    Ok(())
  }

  async fn commit_tx(&mut self) -> Result<Option<QueryResultsWithMetadata>> {
    if !matches!(self.task, Some(MySqlTask::TxPending(_))) {
      Ok(None)
    } else {
      match self.task.take() {
        Some(MySqlTask::TxPending(b)) => {
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
    if let Some(MySqlTask::TxPending(b)) = self.task.take() {
      b.0.rollback().await?;
    }
    self.querying_conn = None;
    self.querying_pid = None;
    Ok(())
  }

  fn start_load_menu(&self) -> Result<JoinHandle<Result<Rows>>> {
    let pool = self.pool.clone().unwrap();
    Ok(tokio::spawn(async move {
      match query_with_pool(pool.clone(), MENU_QUERY.to_owned()).await {
        Ok(rows) => Ok(rows),
        Err(e) => {
          log::warn!("Falling back to simple MySQL menu query: {e:?}");
          query_with_pool(pool, SIMPLE_MENU_QUERY.to_owned()).await
        },
      }
    }))
  }

  fn start_load_columns(
    &self,
    tables: Vec<TableRef>,
  ) -> Result<JoinHandle<Result<Vec<TableColumns>>>> {
    let pool = self.pool.clone().unwrap();
    Ok(tokio::spawn(async move {
      let mut output = Vec::with_capacity(tables.len());
      for table in tables {
        let schema = table.schema.replace('\'', "''");
        let name = table.table.replace('\'', "''");
        let query = format!(
          "select column_name, data_type from information_schema.columns where table_schema = '{schema}' and table_name = '{name}' order by ordinal_position"
        );
        output.push(table_columns_from_rows(table, query_with_pool(pool.clone(), query).await?));
      }
      Ok(output)
    }))
  }

  fn preview_rows_query(&self, schema: &str, table: &str) -> String {
    format!("select * from `{schema}`.`{table}` limit 100")
  }

  fn preview_columns_query(&self, schema: &str, table: &str) -> String {
    format!(
      "select column_name, data_type, is_nullable, column_default, extra, column_comment
        from information_schema.columns
        where table_schema = '{schema}' and table_name = '{table}'
        order by ordinal_position"
    )
  }

  fn preview_constraints_query(&self, schema: &str, table: &str) -> String {
    format!(
      "select constraint_name, constraint_type, enforced,
        group_concat(column_name order by ordinal_position) as column_names
        from information_schema.table_constraints
        join information_schema.key_column_usage using (constraint_schema, constraint_name, table_schema, table_name)
        where table_schema = '{schema}' and table_name = '{table}'
        group by constraint_name, constraint_type, enforced
        order by constraint_type, constraint_name"
    )
  }

  fn preview_indexes_query(&self, schema: &str, table: &str) -> String {
    format!(
      "select index_name, column_name, non_unique, seq_in_index, index_type
        from information_schema.statistics
        where table_schema = '{schema}' and table_name = '{table}'
        order by index_name, seq_in_index"
    )
  }

  fn preview_policies_query(&self, schema: &str, table: &str) -> String {
    "select 'MySQL does not support row-level security policies' as message".to_owned()
  }

  fn preview_view_definition_query(&self, schema: &str, view: &str, materialized: bool) -> String {
    if materialized {
      return "select 'MySQL does not support materialized views' as message".to_owned();
    }
    format!(
      "select view_definition
        from information_schema.views
        where table_schema = '{schema}' and table_name = '{view}'"
    )
  }

  fn preview_function_definition_query(&self, schema: &str, function: &str) -> String {
    format!(
      "select routine_definition
        from information_schema.routines
        where routine_type = 'FUNCTION'
          and routine_schema = '{schema}'
          and routine_name = '{function}'"
    )
  }
}

impl MySqlDriver {
  pub fn new() -> Self {
    Self { pool: None, task: None, querying_conn: None, querying_pid: None }
  }

  fn build_connection_opts(
    args: crate::cli::Cli,
  ) -> Result<<<sqlx::MySql as sqlx::Database>::Connection as sqlx::Connection>::Options> {
    let ssl_required = args.ssl_required;
    let opts = match args.connection_url {
      Some(url) => {
        let mut opts = MySqlConnectOptions::from_str(url.trim().trim_start_matches("jdbc:"))?;
        if args.enable_cleartext_plugin {
          opts = opts.enable_cleartext_plugin(true);
        }
        opts
      },
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
          let password =
            rpassword::prompt_password(format!("password for user {}: ", opts.get_username()))
              .unwrap();
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

        // Cleartext plugin
        if args.enable_cleartext_plugin {
          opts = opts.enable_cleartext_plugin(true);
        }

        opts
      },
    };

    Ok(if ssl_required {
      match opts.get_ssl_mode() {
        MySqlSslMode::VerifyCa | MySqlSslMode::VerifyIdentity => opts,
        _ => opts.ssl_mode(MySqlSslMode::Required),
      }
    } else {
      opts
    })
  }
}

async fn query_with_pool(pool: Arc<sqlx::Pool<MySql>>, query: String) -> Result<Rows> {
  query_with_stream(&*pool.clone(), &query).await
}

async fn query_with_conn(conn: &mut MySqlConnection, query: String) -> Result<Rows> {
  query_with_stream(conn, &query).await
}

fn spawn_query_task(
  conn: Arc<Mutex<PoolConnection<MySql>>>,
  first_query: String,
  statement_type: Option<Statement>,
) -> QueryTask {
  tokio::spawn(async move {
    let results = query_with_conn(conn.lock().await.as_mut(), first_query.clone()).await;
    match results {
      Ok(ref rows) => {
        log::info!("{:?} rows, {:?} affected", rows.rows.len(), rows.rows_affected);
      },
      Err(ref e) => {
        log::error!("{e:?}");
      },
    };
    QueryResultsWithMetadata::new(results, statement_type.clone())
  })
}

#[derive(Clone, Copy)]
enum ScriptState {
  Normal,
  SingleQuoted,
  DoubleQuoted,
  Backtick,
  LineComment,
  BlockComment,
}

fn preprocess_delimiter_script(query: String, mut no_backslash_escapes: bool) -> String {
  let mut delimiter = ";".to_owned();
  let mut statements = Vec::new();
  let mut statement = String::new();
  let mut state = ScriptState::Normal;
  let mut ends_with_line_comment = false;
  let mut saw_directive = false;
  let mut index = 0;

  while index < query.len() {
    if matches!(state, ScriptState::Normal) && (index == 0 || query.as_bytes()[index - 1] == b'\n')
    {
      let line_end = query[index..].find('\n').map_or(query.len(), |offset| index + offset);
      if let Some(new_delimiter) = delimiter_directive(&query[index..line_end]) {
        saw_directive = true;
        if let Some(new_delimiter) = new_delimiter {
          delimiter = new_delimiter.to_owned();
        }
        index = if line_end < query.len() { line_end + 1 } else { line_end };
        continue;
      }
    }

    let remaining = &query[index..];
    if matches!(state, ScriptState::Normal)
      && !delimiter.is_empty()
      && remaining.starts_with(&delimiter)
    {
      if !statement.trim().is_empty() {
        if let Some(enabled) = no_backslash_escapes_setting(&statement) {
          no_backslash_escapes = enabled;
        }
        statements.push((statement.trim().to_owned(), ends_with_line_comment));
      }
      statement.clear();
      ends_with_line_comment = false;
      index += delimiter.len();
      continue;
    }

    let mut chars = remaining.chars();
    let current = chars.next().unwrap();
    let next = chars.next();
    let third = chars.next();

    match state {
      ScriptState::Normal => match (current, next) {
        ('\'', _) => {
          state = ScriptState::SingleQuoted;
          ends_with_line_comment = false;
          statement.push(current);
          index += current.len_utf8();
        },
        ('"', _) => {
          state = ScriptState::DoubleQuoted;
          ends_with_line_comment = false;
          statement.push(current);
          index += current.len_utf8();
        },
        ('`', _) => {
          state = ScriptState::Backtick;
          ends_with_line_comment = false;
          statement.push(current);
          index += current.len_utf8();
        },
        ('#', _) => {
          state = ScriptState::LineComment;
          ends_with_line_comment = true;
          statement.push(current);
          index += current.len_utf8();
        },
        ('-', Some('-')) if third.is_some_and(|c| c.is_whitespace() || c.is_control()) => {
          state = ScriptState::LineComment;
          ends_with_line_comment = true;
          statement.push_str("--");
          index += 2;
        },
        ('/', Some('*')) => {
          state = ScriptState::BlockComment;
          ends_with_line_comment = false;
          statement.push_str("/*");
          index += 2;
        },
        _ => {
          if !current.is_whitespace() {
            ends_with_line_comment = false;
          }
          statement.push(current);
          index += current.len_utf8();
        },
      },
      ScriptState::SingleQuoted | ScriptState::DoubleQuoted | ScriptState::Backtick => {
        let quote = match state {
          ScriptState::SingleQuoted => '\'',
          ScriptState::DoubleQuoted => '"',
          ScriptState::Backtick => '`',
          _ => unreachable!(),
        };
        statement.push(current);
        index += current.len_utf8();

        if current == '\\' && !no_backslash_escapes && !matches!(state, ScriptState::Backtick) {
          if let Some(next) = next {
            statement.push(next);
            index += next.len_utf8();
          }
        } else if current == quote {
          if next == Some(quote) {
            statement.push(quote);
            index += quote.len_utf8();
          } else {
            state = ScriptState::Normal;
          }
        }
      },
      ScriptState::LineComment => {
        statement.push(current);
        index += current.len_utf8();
        if current == '\n' {
          state = ScriptState::Normal;
        }
      },
      ScriptState::BlockComment => {
        if current == '*' && next == Some('/') {
          statement.push_str("*/");
          index += 2;
          state = ScriptState::Normal;
        } else {
          statement.push(current);
          index += current.len_utf8();
        }
      },
    }
  }

  if !saw_directive {
    return query;
  }

  if !statement.trim().is_empty() {
    statements.push((statement.trim().to_owned(), ends_with_line_comment));
  }

  let mut normalized = String::new();
  for (index, (statement, ends_with_line_comment)) in statements.into_iter().enumerate() {
    if index > 0 {
      normalized.push('\n');
    }
    normalized.push_str(&statement);
    if ends_with_line_comment {
      normalized.push('\n');
    }
    normalized.push(';');
  }
  normalized
}

fn no_backslash_escapes_setting(statement: &str) -> Option<bool> {
  const SET: &str = "SET";

  let statement = statement.trim_start();
  let keyword = statement.get(..SET.len())?;
  if !keyword.eq_ignore_ascii_case(SET) {
    return None;
  }

  let remainder = &statement[SET.len()..];
  if !remainder.starts_with(char::is_whitespace) {
    return None;
  }

  let (variable, value) = remainder.trim_start().split_once('=')?;
  let variable = variable.trim();
  let variable = variable.strip_prefix("@@").unwrap_or(variable).trim_start();
  let variable = if let Some((scope, variable)) = variable.split_once('.') {
    if scope.eq_ignore_ascii_case("SESSION") || scope.eq_ignore_ascii_case("LOCAL") {
      variable
    } else {
      return None;
    }
  } else if let Some((scope, variable)) = variable.split_once(char::is_whitespace) {
    if scope.eq_ignore_ascii_case("SESSION") || scope.eq_ignore_ascii_case("LOCAL") {
      variable.trim_start()
    } else {
      return None;
    }
  } else {
    variable
  };
  if !variable.eq_ignore_ascii_case("sql_mode") {
    return None;
  }

  let value = value.trim();
  let quote = value.chars().next()?;
  if !matches!(quote, '\'' | '"') || !value.ends_with(quote) {
    return None;
  }
  let modes = &value[quote.len_utf8()..value.len() - quote.len_utf8()];
  Some(sql_mode_has_no_backslash_escapes(modes))
}

fn sql_mode_has_no_backslash_escapes(sql_mode: &str) -> bool {
  sql_mode.split(',').any(|mode| mode.trim().eq_ignore_ascii_case("NO_BACKSLASH_ESCAPES"))
}

async fn session_no_backslash_escapes(conn: &mut MySqlConnection) -> Result<bool> {
  let sql_mode =
    sqlx::query_scalar::<_, String>("SELECT @@SESSION.sql_mode").fetch_one(conn).await?;
  Ok(sql_mode_has_no_backslash_escapes(&sql_mode))
}

fn delimiter_directive(line: &str) -> Option<Option<&str>> {
  const KEYWORD: &str = "DELIMITER";

  let line = line.trim();
  let keyword = line.get(..KEYWORD.len())?;
  if !keyword.eq_ignore_ascii_case(KEYWORD) {
    return None;
  }

  let remainder = &line[KEYWORD.len()..];
  if !remainder.is_empty() && !remainder.starts_with(char::is_whitespace) {
    return None;
  }

  Some(remainder.split_whitespace().next().map(|delimiter| {
    match (delimiter.chars().next(), delimiter.chars().last()) {
      (Some(first), Some(last))
        if delimiter.len() >= first.len_utf8() + last.len_utf8()
          && first == last
          && matches!(first, '\'' | '"') =>
      {
        &delimiter[first.len_utf8()..delimiter.len() - last.len_utf8()]
      },
      _ => delimiter,
    }
  }))
}

fn spawn_tx_task(
  tx: MySqlTransaction,
  first_query: String,
  statement_type: Statement,
  display_statement_type: Statement,
) -> TransactionTask {
  tokio::spawn(async move {
    let (results, tx) = query_with_tx(tx, &first_query).await;
    match results {
      Ok(Either::Left(rows_affected)) => {
        log::info!("{rows_affected:?} rows affected");
        (
          QueryResultsWithMetadata {
            results: Ok(Rows { headers: vec![], rows: vec![], rows_affected: Some(rows_affected) }),
            statement_type: Some(statement_type),
            display_statement_type: Some(display_statement_type),
          },
          tx,
        )
      },
      Ok(Either::Right(rows)) => {
        log::info!("{:?} rows affected", rows.rows_affected);
        (
          QueryResultsWithMetadata::with_display_statement_type(
            Ok(rows),
            Some(statement_type),
            Some(display_statement_type),
          ),
          tx,
        )
      },
      Err(e) => {
        log::error!("{e:?}");
        (
          QueryResultsWithMetadata::with_display_statement_type(
            Err(e),
            Some(statement_type),
            Some(display_statement_type),
          ),
          tx,
        )
      },
    }
  })
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

async fn query_with_tx(
  mut tx: MySqlTransaction,
  query: &str,
) -> (Result<Either<u64, Rows>>, MySqlTransaction)
where
  for<'c> <sqlx::MySql as sqlx::Database>::Arguments<'c>: sqlx::IntoArguments<'c, sqlx::MySql>,
  for<'c> &'c mut <sqlx::MySql as sqlx::Database>::Connection:
    sqlx::Executor<'c, Database = sqlx::MySql>,
{
  let first_query = super::get_first_query(query.to_string(), Driver::MySql);
  match first_query {
    Ok((first_query, statement_type)) => match statement_type {
      statement_type if super::should_stream_tx_results(&statement_type) => {
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

async fn connection_pid(conn: &mut MySqlConnection) -> Result<u64>
where
  for<'c> &'c mut <sqlx::MySql as sqlx::Database>::Connection:
    sqlx::Executor<'c, Database = sqlx::MySql>,
{
  let ipid = sqlx::query_scalar::<_, i64>("SELECT CONNECTION_ID()").fetch_one(&mut *conn).await;
  if let Ok(pid) = ipid {
    return Ok(pid as u64);
  }
  let upid = sqlx::query_scalar::<_, u64>("SELECT CONNECTION_ID()").fetch_one(&mut *conn).await;
  if let Ok(pid) = upid {
    return Ok(pid);
  }
  let spid = sqlx::query_scalar::<_, String>("SELECT CONNECTION_ID()").fetch_one(conn).await?;
  spid.parse().map_err(|e| {
    eyre::Report::new(e)
      .wrap_err("Failed to parse connection PID after deserializing it as a string")
  })
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
fn parse_value(
  row: &<MySql as sqlx::Database>::Row,
  col: &<MySql as sqlx::Database>::Column,
) -> Option<Value> {
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
    "VARCHAR" | "CHAR" | "TEXT" | "BINARY" => {
      Some(row.try_get::<String, usize>(col.ordinal()).map_or(
        Value { parse_error: true, string: "_ERROR_".to_string(), is_null: false },
        |received| Value { parse_error: false, string: received.to_string(), is_null: false },
      ))
    },
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
  use clap::Parser;
  use sqlparser::{ast::Statement, dialect::MySqlDialect, parser::ParserError};
  use sqlx::mysql::MySqlSslMode;

  use super::*;
  use crate::database::{ExecutionType, ParseError, get_execution_type, get_first_query};

  #[test]
  fn ssl_required_overrides_url_ssl_mode() {
    let args = crate::cli::Cli::parse_from([
      "rainfrog",
      "--url",
      "mysql://localhost/mysql?sslmode=disabled",
      "--ssl-required",
    ]);
    let opts = MySqlDriver::build_connection_opts(args).unwrap();
    assert!(matches!(opts.get_ssl_mode(), MySqlSslMode::Required));
  }

  #[test]
  fn ssl_required_preserves_stricter_url_ssl_modes() {
    for (mode, expected) in
      [("verify_ca", MySqlSslMode::VerifyCa), ("verify_identity", MySqlSslMode::VerifyIdentity)]
    {
      let args = crate::cli::Cli::parse_from([
        "rainfrog",
        "--url",
        &format!("mysql://localhost/mysql?sslmode={mode}"),
        "--ssl-required",
      ]);
      let opts = MySqlDriver::build_connection_opts(args).unwrap();
      assert!(
        matches!(
          (opts.get_ssl_mode(), expected),
          (MySqlSslMode::VerifyCa, MySqlSslMode::VerifyCa)
            | (MySqlSslMode::VerifyIdentity, MySqlSslMode::VerifyIdentity)
        ),
        "mode {mode} was not preserved"
      );
    }
  }

  #[test]
  fn ssl_required_applies_to_structured_options() {
    let args = crate::cli::Cli::parse_from([
      "rainfrog",
      "--driver",
      "mysql",
      "--username",
      "user",
      "--password",
      "password",
      "--host",
      "localhost",
      "--port",
      "3306",
      "--database",
      "mysql",
      "--ssl-required",
    ]);
    let opts = MySqlDriver::build_connection_opts(args).unwrap();
    assert!(matches!(opts.get_ssl_mode(), MySqlSslMode::Required));
  }

  #[test]
  fn preprocesses_mysql_delimiter_script() {
    let query = r#"DELIMITER $$
CREATE PROCEDURE hello_world()
BEGIN
  SELECT 'Hello World';
END $$
DELIMITER ;"#;

    assert_eq!(
      preprocess_delimiter_script(query.to_owned(), false),
      "CREATE PROCEDURE hello_world()\nBEGIN\n  SELECT 'Hello World';\nEND;"
    );
  }

  #[test]
  fn preprocesses_quoted_mysql_delimiters() {
    let query = r#"DELIMITER "$$"
SELECT 1$$
DELIMITER ";""#;

    assert_eq!(preprocess_delimiter_script(query.to_owned(), false), "SELECT 1;");
  }

  #[test]
  fn custom_delimiters_ignore_quoted_and_commented_tokens() {
    let query = r#"DELIMITER //
CREATE PROCEDURE delimiter_examples()
BEGIN
  SELECT 'single // ;', "double // ;", `identifier//name`;
  -- a // line comment
  # another // line comment
  /* a // block comment */
  SELECT 'escaped \'// value', 'doubled ''// value';
END//
DELIMITER ;
SELECT 1;"#;

    assert_eq!(
      preprocess_delimiter_script(query.to_owned(), false),
      r#"CREATE PROCEDURE delimiter_examples()
BEGIN
  SELECT 'single // ;', "double // ;", `identifier//name`;
  -- a // line comment
  # another // line comment
  /* a // block comment */
  SELECT 'escaped \'// value', 'doubled ''// value';
END;
SELECT 1;"#
    );
  }

  #[test]
  fn backslashes_do_not_escape_backticks() {
    let query = r#"DELIMITER $$
SELECT 1 AS `p\`$$
DELIMITER ;
SELECT 2;"#;

    assert_eq!(
      preprocess_delimiter_script(query.to_owned(), false),
      r#"SELECT 1 AS `p\`;
SELECT 2;"#
    );
  }

  #[test]
  fn no_backslash_escapes_mode_does_not_escape_closing_quotes() {
    let query = r#"SET sql_mode='NO_BACKSLASH_ESCAPES';
DELIMITER $$
CREATE PROCEDURE backslash_literal()
BEGIN
  SELECT 'abc\';
END$$
DELIMITER ;"#;

    assert_eq!(
      preprocess_delimiter_script(query.to_owned(), false),
      r#"SET sql_mode='NO_BACKSLASH_ESCAPES';
CREATE PROCEDURE backslash_literal()
BEGIN
  SELECT 'abc\';
END;"#
    );
  }

  #[test]
  fn connection_no_backslash_escapes_mode_does_not_escape_closing_quotes() {
    let query = r#"DELIMITER $$
SELECT 'abc\'$$
DELIMITER ;
SELECT 1;"#;

    assert_eq!(
      preprocess_delimiter_script(query.to_owned(), true),
      r#"SELECT 'abc\';
SELECT 1;"#
    );
  }

  #[test]
  fn script_without_delimiter_directive_is_unchanged() {
    let query = "  SELECT 'preserve formatting';\r\n\r\nSELECT 2;  ";

    assert_eq!(preprocess_delimiter_script(query.to_owned(), false), query);
    assert_eq!(preprocess_delimiter_script(String::new(), false), "");
    assert_eq!(preprocess_delimiter_script(" \n\t".to_owned(), false), " \n\t");
  }

  #[test]
  fn blank_delimiter_directive_is_ignored_without_creating_statements() {
    assert_eq!(preprocess_delimiter_script("DELIMITER\nSELECT 1;".to_owned(), false), "SELECT 1;");
    assert_eq!(preprocess_delimiter_script("  delimiter  \n".to_owned(), false), "");
  }

  #[test]
  fn terminates_statements_after_trailing_line_comments() {
    let query = "DELIMITER $$\nSELECT 1 -- first\n$$\nSELECT 2 # second\n$$";

    assert_eq!(
      preprocess_delimiter_script(query.to_owned(), false),
      "SELECT 1 -- first\n;\nSELECT 2 # second\n;"
    );
  }

  #[test]
  fn double_dash_without_whitespace_is_not_a_comment() {
    let query = "DELIMITER $$\nSELECT 1--2$$";

    assert_eq!(preprocess_delimiter_script(query.to_owned(), false), "SELECT 1--2;");
  }

  #[test]
  fn test_get_first_query() {
    type TestCase = (&'static str, Result<(String, Box<dyn Fn(Statement) -> bool>), ParseError>);

    let test_cases: Vec<TestCase> = vec![
      // single query
      (
        "SELECT * FROM users;",
        Ok(("SELECT * FROM users".to_string(), Box::new(|s| matches!(s, Statement::Query(_))))),
      ),
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
      (
        "select *\nfrom users;",
        Ok(("SELECT * FROM users".to_owned(), Box::new(|s| matches!(s, Statement::Query(_))))),
      ),
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
        Ok((
          "DELETE FROM users WHERE id = 1".to_owned(),
          Box::new(|s| matches!(s, Statement::Delete { .. })),
        )),
      ),
      // drop
      (
        "DROP TABLE users",
        Ok(("DROP TABLE users".to_owned(), Box::new(|s| matches!(s, Statement::Drop { .. })))),
      ),
      // explain
      (
        "EXPLAIN SELECT * FROM users",
        Ok((
          "EXPLAIN SELECT * FROM users".to_owned(),
          Box::new(|s| matches!(s, Statement::Explain { .. })),
        )),
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
        (
          Err(ParseError::MoreThanOneStatement(msg)),
          Err(ParseError::MoreThanOneStatement(expected_msg)),
        ) => {
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
        "Failed for query: {query}"
      );
    }
  }
}
