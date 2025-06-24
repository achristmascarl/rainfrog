use std::{
  io::{self, Write as _},
  str::FromStr,
  sync::Arc,
};

use async_trait::async_trait;
use color_eyre::eyre::Result;
use oracle::Connection;
use sqlparser::ast::Statement;

use crate::cli::Driver;

use super::{Database, DbTaskResult, Header, QueryResultsWithMetadata, QueryTask, Rows};

pub struct OracleConnectOptions {
  user: Option<String>,
  password: Option<String>,
  host: String,
  port: Option<u16>,
  database: String,
}
impl OracleConnectOptions {
  fn new() -> Self {
    Self { user: None, password: None, host: "localhost".to_string(), port: Some(1521), database: "XE".to_string() }
  }

  pub fn get_connection_string(&self) -> String {
    let port = if let Some(port) = self.port { port } else { 1521u16 };
    format!("//{}:{}/{}", self.host, port, self.database)
  }
  pub fn get_connection_options(&self) -> std::result::Result<(String, String, String), String> {
    let user = self.user.clone().ok_or("User is required for Oracle connection".to_string())?;
    let password = self.password.clone().ok_or("Password is required for Oracle connection".to_string())?;
    let connection_string = self.get_connection_string();
    Ok((user, password, connection_string))
  }
}

impl FromStr for OracleConnectOptions {
  type Err = String;

  fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
    let s = s.trim().trim_start_matches("jdbc:oracle:thin:");
    let (is_easy_connect, (auth_part, host_part)) = if s.contains("@//") {
      (true, s.split_once("@//").ok_or("Invalid Oracle Easy Connect connection string format".to_string())?)
    } else if s.contains("@") {
      (false, s.split_once('@').ok_or("Invalid Oracle SID connection string format".to_string())?)
    } else {
      return Err("Invalid Oracle connection string format".to_string());
    };

    let (user, password) = if auth_part.contains('/') {
      let (user, password) = auth_part.split_once('/').ok_or("Invalid Oracle connection string format".to_string())?;
      (Some(user.to_string()), Some(password.to_string()))
    } else if !auth_part.is_empty() {
      (Some(auth_part.to_string()), None)
    } else {
      (None, None)
    };

    let (host, port, database) = if is_easy_connect {
      let (host_port, database) =
        host_part.split_once('/').ok_or("Invalid Oracle Easy Connect connection string format".to_string())?;

      let (host, port) = if host_port.contains(':') {
        let (host, port) =
          host_port.split_once(':').ok_or("Invalid Oracle Easy Connect connection string format".to_string())?;
        let port = port.parse().map_err(|_| "Invalid port")?;
        (host.to_string(), Some(port))
      } else {
        (host_port.to_string(), None)
      };

      (host, port, database.to_string())
    } else {
      let parts = host_part.split(':').collect::<Vec<_>>();
      if parts.len() == 2 {
        (parts[0].to_string(), None, parts[1].to_string())
      } else if parts.len() == 3 {
        let port = parts[1].parse().map_err(|_| "Invalid port")?;
        (parts[0].to_string(), Some(port), parts[2].to_string())
      } else {
        return Err("Invalid Oracle SID connection string format".to_string());
      }
    };

    Ok(OracleConnectOptions { user, password, host, port, database })
  }
}

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

  fn build_connection_opts(args: crate::cli::Cli) -> Result<OracleConnectOptions> {
    match args.connection_url {
      Some(url) => {
        let mut opts = OracleConnectOptions::from_str(&url).map_err(|e| color_eyre::eyre::eyre!(e))?;

        // Username
        if opts.user.is_none() {
          if let Some(user) = args.user {
            opts.user = Some(user);
          } else {
            let mut user = String::new();
            print!("username: ");
            io::stdout().flush()?;
            io::stdin().read_line(&mut user)?;
            let user = user.trim();
            if !user.is_empty() {
              opts.user = Some(user.to_string());
            }
          }
        }

        // Password
        if opts.password.is_none() {
          if let Some(password) = args.password {
            opts.password = Some(password);
          } else {
            let password =
              rpassword::prompt_password(format!("password for user {}: ", opts.user.as_ref().unwrap())).unwrap();
            let password = password.trim();
            if !password.is_empty() {
              opts.password = Some(password.to_string());
            }
          }
        }

        Ok(opts)
      },
      None => {
        let mut opts = OracleConnectOptions::new();

        // Username
        if let Some(user) = args.user {
          opts.user = Some(user);
        } else {
          let mut user = String::new();
          print!("username: ");
          io::stdout().flush()?;
          io::stdin().read_line(&mut user)?;
          let user = user.trim();
          if !user.is_empty() {
            opts.user = Some(user.to_string());
          }
        }

        // Password
        if let Some(password) = args.password {
          opts.password = Some(password);
        } else {
          let password =
            rpassword::prompt_password(format!("password for user {}: ", opts.user.as_ref().unwrap())).unwrap();
          let password = password.trim();
          if !password.is_empty() {
            opts.password = Some(password.to_string());
          }
        }

        // Host
        if let Some(host) = args.host {
          opts.host = host;
        } else {
          let mut host = String::new();
          print!("host (ex. localhost): ");
          io::stdout().flush()?;
          io::stdin().read_line(&mut host)?;
          let host = host.trim();
          if !host.is_empty() {
            opts.host = host.to_string();
          }
        }

        // Port
        if let Some(port) = args.port {
          opts.port = Some(port);
        } else {
          let mut port = String::new();
          print!("port (ex. 1521): ");
          io::stdout().flush()?;
          io::stdin().read_line(&mut port)?;
          let port = port.trim();
          if !port.is_empty() {
            opts.port = Some(port.parse()?);
          }
        }

        // Database
        if let Some(database) = args.database {
          opts.database = database;
        } else {
          let mut database = String::new();
          print!("database (ex. mydb): ");
          io::stdout().flush()?;
          io::stdin().read_line(&mut database)?;
          let database = database.trim();
          if !database.is_empty() {
            opts.database = database.to_string();
          }
        }

        Ok(opts)
      },
    }
  }
}

#[async_trait(?Send)]
impl Database for OracleDriver {
  async fn init(&mut self, args: crate::cli::Cli) -> Result<()> {
    let connection_opts = Self::build_connection_opts(args)?;

    let (user, password, connection_string) =
      connection_opts.get_connection_options().map_err(|e| color_eyre::eyre::eyre!(e))?;
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
