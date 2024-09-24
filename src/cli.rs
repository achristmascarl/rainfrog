use std::path::PathBuf;

use clap::Parser;

use crate::utils::version;

#[derive(Parser, Debug, Clone)]
#[command(author, version = version(), about)]
pub struct Cli {
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
  pub database: String,
}
