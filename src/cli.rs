use std::{
  io::{self, Write},
  str::FromStr,
};

use clap::Parser;
use color_eyre::eyre::{self, Result};
use serde::Deserialize;

use crate::{
  config::{Config, DatabaseConnection},
  utils::version,
};

#[derive(Parser, Debug, Clone)]
#[command(author, version = version(), about)]
pub struct Cli {
  #[arg(
    short = 'M',
    long = "mouse",
    value_name = "MOUSE_MODE",
    help = "Whether to enable mouse event support. If enabled, your terminal's default mouse event handling will not work."
  )]
  pub mouse_mode: Option<bool>,

  #[arg(
    short = 'u',
    long = "url",
    value_name = "URL",
    help = "Full connection URL for the database, e.g. postgres://username:password@localhost:5432/dbname"
  )]
  pub connection_url: Option<String>,

  #[arg(long = "username", value_name = "USERNAME", help = "Username for database connection")]
  pub user: Option<String>,

  #[arg(long = "password", value_name = "PASSWORD", help = "Password for database connection")]
  pub password: Option<String>,

  #[arg(long = "host", value_name = "HOST", help = "Host for database connection (ex. localhost)")]
  pub host: Option<String>,

  #[arg(long = "port", value_name = "PORT", help = "Port for database connection (ex. 5432)")]
  pub port: Option<u16>,

  #[arg(long = "database", value_name = "DATABASE", help = "Name of database for connection (ex. postgres)")]
  pub database: Option<String>,

  #[arg(long = "driver", value_name = "DRIVER", help = "Driver for database connection (ex. postgres)")]
  pub driver: Option<Driver>,

  #[arg(skip)]
  pub connection_name: Option<String>,
}

#[derive(Parser, Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub enum Driver {
  #[serde(alias = "postgres", alias = "POSTGRES")]
  Postgres,
  #[serde(alias = "mysql", alias = "MYSQL")]
  MySql,
  #[serde(alias = "sqlite", alias = "SQLITE")]
  Sqlite,
  #[serde(alias = "oracle", alias = "ORACLE")]
  Oracle,
  #[cfg(feature = "duckdb")]
  #[serde(alias = "duckdb", alias = "DUCKDB")]
  DuckDb,
}

impl FromStr for Driver {
  type Err = eyre::Report;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    match s.to_lowercase().as_str() {
      "postgres" | "postgresql" => Ok(Driver::Postgres),
      "mysql" => Ok(Driver::MySql),
      "sqlite" => Ok(Driver::Sqlite),
      "oracle" => Ok(Driver::Oracle),
      #[cfg(feature = "duckdb")]
      "duckdb" => Ok(Driver::DuckDb),
      _ => Err(eyre::Report::msg("Invalid driver")),
    }
  }
}

pub fn extract_driver_from_url(url: &str) -> Result<Driver> {
  let url = url.trim();
  if url.starts_with("jdbc:") {
    if let Some(driver_part) = url.split(':').nth(1) {
      driver_part.to_lowercase().parse()
    } else {
      Err(eyre::Report::msg("Invalid connection URL format"))
    }
  } else if let Some(pos) = url.find("://") {
    url[..pos].to_lowercase().parse()
  } else if url.ends_with(".duckdb") || url.ends_with(".ddb") {
    #[cfg(feature = "duckdb")]
    {
      return Ok(Driver::DuckDb);
    }
    #[allow(unreachable_code)] // because of cfg above
    Err(eyre::Report::msg("DuckDb is not supported on this architecture"))
  } else if url.ends_with(".sqlite") || url.ends_with(".sqlite3") {
    Ok(Driver::Sqlite)
  } else if url.ends_with(".db") {
    Err(eyre::Report::msg("File extension is ambiguous, please specify driver explicitly"))
  } else {
    Err(eyre::Report::msg("Invalid connection URL format"))
  }
}

pub fn extract_port_and_database_from_url(url: &str) -> Option<(u16, String)> {
  let mut url = url.trim();
  if let Some(jdbc_url) = url.strip_prefix("jdbc:") {
    url = jdbc_url;
  }

  if let Some((_, rest)) = url.split_once("://")
    && let Some((authority, path)) = rest.split_once('/')
  {
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, host_port)| host_port).trim_start_matches('/');
    let port = if host_port.starts_with('[') { host_port.split_once("]:")?.1 } else { host_port.rsplit_once(':')?.1 }
      .parse()
      .ok()?;

    let database = path.split('?').next()?.split('#').next()?.trim_start_matches('/');
    if database.is_empty() {
      return None;
    }

    return Some((port, database.to_string()));
  }

  let (_, rest) = url.rsplit_once('@')?;
  let rest = rest.trim_start_matches('/');

  if let Some((host_port, database)) = rest.split_once('/') {
    let port = if host_port.starts_with('[') { host_port.split_once("]:")?.1 } else { host_port.rsplit_once(':')?.1 }
      .parse()
      .ok()?;

    if database.is_empty() {
      return None;
    }

    return Some((port, database.to_string()));
  }

  let parts: Vec<&str> = rest.split(':').collect();
  if parts.len() == 3 {
    let port = parts[1].parse().ok()?;
    if parts[2].is_empty() {
      return None;
    }
    return Some((port, parts[2].to_string()));
  }

  None
}

pub fn prompt_for_database_selection(config: &Config) -> Result<Option<(DatabaseConnection, String)>> {
  match config.db.len() {
    0 => Ok(None),
    1 => Ok(Some(config.db.iter().map(|(name, db)| (db.clone(), name.to_string())).next().unwrap())),
    _ => {
      let defaults: Vec<_> = config.db.iter().filter(|(_, d)| d.default).collect();
      match defaults.len() {
        0 => {
          let mut db_names: Vec<&str> = config.db.keys().map(|n| n.as_str()).collect();
          db_names.sort();
          for (i, name) in db_names.iter().enumerate() {
            println!("[{i}] {name}");
          }
          print!("Input index of desired database: ");

          let mut index = String::new();
          io::stdout().flush()?;
          io::stdin().read_line(&mut index)?;

          let index: usize = index.trim().parse()?;

          if index >= db_names.len() {
            Err(eyre::Report::msg("Database index not recognized"))
          } else {
            let name = db_names[index].to_string();
            Ok(Some((config.db[&name].clone(), name)))
          }
        },
        1 => Ok(Some((defaults[0].1.clone(), defaults[0].0.clone()))),
        _ => Err(eyre::Report::msg("Multiple default database connections defined")),
      }
    },
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn extracts_driver_from_standard_urls() {
    let cases = [
      ("postgres://username:password@localhost:5432/dbname", Driver::Postgres),
      ("postgresql://readonly@reports.example.com/reporting?sslmode=require", Driver::Postgres),
      ("postgres://user:pass@[2001:db8::1]:5432/app", Driver::Postgres),
      ("postgresql://user@/analytics?host=/var/run/postgresql", Driver::Postgres),
      ("POSTGRES://localhost/dbname", Driver::Postgres),
      ("mysql://localhost/dbname", Driver::MySql),
      ("mysql://app:pw@192.168.10.10:3307/metrics?useSSL=false", Driver::MySql),
      ("mysql://reader:secret@db.example.com/app?charset=utf8mb4", Driver::MySql),
      ("sqlite:///tmp/data.sqlite", Driver::Sqlite),
      ("sqlite:///var/lib/sqlite/app.sqlite3", Driver::Sqlite),
      ("sqlite://localhost/var/data.sqlite?mode=ro", Driver::Sqlite),
      ("oracle://scott:tiger@//prod-db.example.com:1521/ORCLPDB1", Driver::Oracle),
      ("oracle://user:pass@db-host/service_name", Driver::Oracle),
      #[cfg(feature = "duckdb")]
      ("duckdb:///var/tmp/cache.duckdb", Driver::DuckDb),
    ];

    for (url, expected) in cases {
      let actual = extract_driver_from_url(url).unwrap_or_else(|err| panic!("url: {url}, err: {err}"));
      assert_eq!(actual, expected, "url: {url}");
    }
  }

  #[test]
  fn extracts_driver_from_jdbc_urls() {
    let cases = [
      ("jdbc:postgresql://localhost:5432/dbname", Driver::Postgres),
      ("jdbc:postgresql://readonly@reports.example.com:5432/reporting?sslmode=require", Driver::Postgres),
      ("jdbc:mysql://localhost:3306/dbname", Driver::MySql),
      ("jdbc:mysql:loadbalance://db1.example.com:3306,db2.example.com:3306/app", Driver::MySql),
      ("jdbc:sqlite://localhost/path", Driver::Sqlite),
      ("jdbc:sqlite:/var/lib/sqlite/cache.sqlite3", Driver::Sqlite),
      ("jdbc:oracle:thin:@localhost:1521/dbname", Driver::Oracle),
      ("jdbc:oracle:oci:@//prod-host:1521/ORCLCDB.localdomain", Driver::Oracle),
      #[cfg(feature = "duckdb")]
      ("jdbc:duckdb:/var/lib/duckdb/cache.duckdb", Driver::DuckDb),
    ];

    for (url, expected) in cases {
      let actual = extract_driver_from_url(url).unwrap_or_else(|err| panic!("url: {url}, err: {err}"));
      assert_eq!(actual, expected, "url: {url}");
    }
  }

  #[test]
  fn extracts_driver_from_file_extensions() {
    let sqlite_paths = ["/tmp/app.sqlite", "/tmp/app.sqlite3", "./relative/state.sqlite", r"C:\data\inventory.sqlite3"];
    for path in sqlite_paths {
      assert_eq!(
        extract_driver_from_url(path).unwrap_or_else(|err| panic!("url: {path}, err: {err}")),
        Driver::Sqlite,
        "url: {path}"
      );
    }

    #[cfg(feature = "duckdb")]
    {
      let duckdb_paths = ["/tmp/data.duckdb", "/tmp/data.ddb", "./var/cache/session.duckdb"];
      for path in duckdb_paths {
        assert_eq!(
          extract_driver_from_url(path).unwrap_or_else(|err| panic!("url: {path}, err: {err}")),
          Driver::DuckDb,
          "url: {path}"
        );
      }
    }

    #[cfg(not(feature = "duckdb"))]
    {
      assert!(extract_driver_from_url("/tmp/data.duckdb").is_err());
    }

    let err = extract_driver_from_url("/tmp/unknown.db").unwrap_err();
    assert!(err.to_string().contains("ambiguous"));
  }

  #[test]
  fn trims_whitespace_before_parsing() {
    let cases = [
      ("  mysql://user@localhost/db  ", Driver::MySql),
      ("\tpostgres://readonly@reports/db\n", Driver::Postgres),
      (" \nsqlite:///tmp/cache.sqlite3\t", Driver::Sqlite),
    ];

    for (url, expected) in cases {
      let actual = extract_driver_from_url(url).unwrap_or_else(|err| panic!("url: {url:?}, err: {err}"));
      assert_eq!(actual, expected, "url: {url:?}");
    }
  }

  #[test]
  fn errors_on_invalid_format() {
    for url in ["localhost:5432/db", "postgresql:/localhost/db", "oracle//prod-host:1521/service"] {
      let err = extract_driver_from_url(url).unwrap_err();
      assert!(err.to_string().contains("Invalid connection URL format"), "Unexpected error for {url}: {err}");
    }
  }

  #[test]
  fn extracts_port_and_database_from_standard_urls() {
    let cases = [
      ("postgres://username:password@localhost:5432/dbname", Some((5432, "dbname".to_string()))),
      ("mysql://reader:secret@db.example.com:3307/app?charset=utf8mb4", Some((3307, "app".to_string()))),
      ("oracle://scott:tiger@//prod-db.example.com:1521/ORCLPDB1", Some((1521, "ORCLPDB1".to_string()))),
      ("sqlite:///tmp/data.sqlite", None),
    ];

    for (url, expected) in cases {
      assert_eq!(extract_port_and_database_from_url(url), expected, "url: {url}");
    }
  }

  #[test]
  fn extracts_port_and_database_from_jdbc_oracle_urls() {
    let cases = [
      ("jdbc:oracle:thin:user/password@//localhost:1521/XE", Some((1521, "XE".to_string()))),
      ("jdbc:oracle:thin:user/password@localhost:1521:XE", Some((1521, "XE".to_string()))),
    ];

    for (url, expected) in cases {
      assert_eq!(extract_port_and_database_from_url(url), expected, "url: {url}");
    }
  }
}
