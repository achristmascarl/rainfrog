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
}

#[derive(Parser, Debug, Clone, Copy, Deserialize)]
pub enum Driver {
  #[serde(alias = "postgres", alias = "POSTGRES")]
  Postgres,
  #[serde(alias = "mysql", alias = "MYSQL")]
  MySql,
  #[serde(alias = "sqlite", alias = "SQLITE")]
  Sqlite,
  #[serde(alias = "oracle", alias = "ORACLE")]
  Oracle,
}

impl FromStr for Driver {
  type Err = eyre::Report;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    match s.to_lowercase().as_str() {
      "postgres" | "postgresql" => Ok(Driver::Postgres),
      "mysql" => Ok(Driver::MySql),
      "sqlite" => Ok(Driver::Sqlite),
      "oracle" => Ok(Driver::Oracle),
      _ => Err(eyre::Report::msg("Invalid driver")),
    }
  }
}

pub fn extract_driver_from_url(url: &str) -> Result<Driver> {
  let url = url.trim();
  if let Some(pos) = url.find("://") {
    url[..pos].to_lowercase().parse()
  } else if url.starts_with("jdbc:oracle:thin") {
    Ok(Driver::Oracle)
  } else {
    Err(eyre::Report::msg("Invalid connection URL format"))
  }
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
