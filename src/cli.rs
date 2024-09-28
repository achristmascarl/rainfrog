use std::{
  io::{self, Write},
  path::PathBuf,
  str::FromStr,
};

use clap::Parser;
use color_eyre::eyre::{self, Result};

use crate::utils::version;

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

#[derive(Parser, Debug, Clone)]
pub enum Driver {
  Postgres,
  Mysql,
  Sqlite,
}

impl FromStr for Driver {
  type Err = eyre::Report;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    match s.to_lowercase().as_str() {
      "postgres" | "postgresql" => Ok(Driver::Postgres),
      "mysql" => Ok(Driver::Mysql),
      "sqlite" => Ok(Driver::Sqlite),
      _ => Err(eyre::Report::msg("Invalid driver")),
    }
  }
}

pub fn extract_driver_from_url(url: &str) -> Result<Driver> {
  let url = url.trim();
  if let Some(pos) = url.find("://") {
    url[..pos].to_lowercase().parse()
  } else {
    Err(eyre::Report::msg("Invalid connection URL format"))
  }
}

pub fn prompt_for_driver() -> Result<Driver> {
  let mut driver = String::new();
  print!("Database driver (postgres, mysql, sqlite): ");
  io::stdout().flush()?;
  io::stdin().read_line(&mut driver)?;
  driver.trim().to_lowercase().parse()
}
